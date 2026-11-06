//! `modengine new [--template <id>] <folder>`: new projects from templates.
//!
//! Templates: `basic` (default: a hero you can move), `empty` (one empty room),
//! `platformer` (a copy of examples/platformer), `topdown` (templates/topdown),
//! `crawler` (examples/crawler: first-person dungeon crawler with card fights) and
//! `showcase` (examples/showcase_assets_s2). The copied ones are embedded at build time
//! (build.rs), so any installed binary can create them. [`NewProject`] can also set the
//! game name, the display size and the palette (sprites are remapped to the nearest
//! colour of the new palette).

use crate::palette::Palette;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/templates.rs"));
}

const GAME_HEAD: &str = r##"[game]
name = "My Mod Engine game"
start_room = "main"

[display]
width = 320
height = 180
scale = 3
letterbox = "#272736"

"##;

/// game.toml of a new project: the default palette is Pear36 (`0`-`9`, `A`-`Z`).
pub fn game_toml() -> String {
    format!("{GAME_HEAD}{}", crate::palette::game_toml_section(&crate::palette::Palette::pear36()))
}

// Pear36 characters: K outline, 4 yellow, F white, C blue.
const HERO: &str = r#"origin = "center"
pixels = ['''
..KKKK..
.K4444K.
K4FKKF4K
K444444K
.KCCCCK.
KCCCCCCK
.KCKKCK.
.KK..KK.
''']
"#;

const HERO_MS: &str = r#"// Objects are declared with `object Name { ... }`.
// `name = value` lines set variables on every new instance;
// `on <event> { ... }` blocks run when that event happens.

object Hero {
    sprite = hero
    move_speed = 2

    on create {
        print("Hello from instance", id)
    }

    on step {
        var dx = input_axis([key.left, key.a], [key.right, key.d])
        var dy = input_axis([key.up, key.w], [key.down, key.s])
        x = clamp(x + dx * move_speed, 4, room_width - 4)
        y = clamp(y + dy * move_speed, 4, room_height - 4)
        if dx != 0 scale_x = dx
    }

    on draw_gui {
        draw_text(4, 4, "Arrow keys / WASD to move")
    }
}
"#;

const ROOM: &str = r##"background = "#323e4f"

[[layer]]
type = "instances"
cell = [16, 16]
legend = { "@" = "Hero" }
map = '''
....................
....................
....................
....................
.........@..........
'''
"##;

const EMPTY_ROOM: &str = r##"background = "#323e4f"

[[layer]]
name = "Instances"
type = "instances"
cell = [16, 16]
"##;

/// A project template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Template {
    /// What `--template` takes.
    pub id: &'static str,
    /// Other accepted ids (Spanish names).
    pub aliases: &'static [&'static str],
    /// Name shown in the editor.
    pub label: &'static str,
    pub description: &'static str,
}

pub const TEMPLATES: &[Template] = &[
    Template { id: "empty", aliases: &["vacio", "vacío"], label: "Vacío", description: "Una room vacía y la paleta elegida: empieza desde cero." },
    Template { id: "basic", aliases: &["basico", "básico", "hero"], label: "Básico", description: "Un personaje que se mueve con flechas / WASD en una room." },
    Template {
        id: "platformer",
        aliases: &["plataformas"],
        label: "Plataformas",
        description: "Coin Climber: gravedad, saltos, enemigos, monedas, 4 rooms y sonidos (copia de examples/platformer).",
    },
    Template { id: "topdown", aliases: &["top-down", "cenital"], label: "Top-down", description: "Vista cenital: muévete en 8 direcciones entre muros y recoge las gemas." },
    Template {
        id: "crawler",
        aliases: &["mazmorra", "cartas", "dungeon", "blobber"],
        label: "Crawler de cartas",
        description: "Cripta de Naipes: mazmorra en primera persona por casillas, pisos procedurales y combate de cartas por turnos (copia de examples/crawler).",
    },
    Template {
        id: "showcase",
        aliases: &["escaparate"],
        label: "Showcase",
        description: "Trayectorias, timelines, curvas de animación y secuencias en acción (examples/showcase_assets_s2).",
    },
];

/// The template with this id or alias (case-insensitive).
pub fn template(id: &str) -> Option<&'static Template> {
    let id = id.trim().to_lowercase();
    TEMPLATES.iter().find(|t| t.id == id || t.aliases.contains(&id.as_str()))
}

/// Files of a template (relative path, contents).
pub fn template_files(id: &str) -> Option<Vec<(String, Cow<'static, [u8]>)>> {
    let t = template(id)?;
    let own = |list: &[(&'static str, &'static [u8])]| list.iter().map(|(r, b)| (r.to_string(), Cow::Borrowed(*b))).collect::<Vec<_>>();
    Some(match t.id {
        "empty" => vec![("game.toml".into(), Cow::Owned(game_toml().into_bytes())), ("rooms/main.toml".into(), Cow::Borrowed(EMPTY_ROOM.as_bytes()))],
        "basic" => vec![
            ("game.toml".into(), Cow::Owned(game_toml().into_bytes())),
            ("sprites/hero.toml".into(), Cow::Borrowed(HERO.as_bytes())),
            ("objects/hero.ms".into(), Cow::Borrowed(HERO_MS.as_bytes())),
            ("rooms/main.toml".into(), Cow::Borrowed(ROOM.as_bytes())),
        ],
        "platformer" => own(embedded::PLATFORMER),
        "topdown" => own(embedded::TOPDOWN),
        "crawler" => own(embedded::CRAWLER),
        "showcase" => own(embedded::SHOWCASE),
        _ => return None,
    })
}

/// What `modengine new` / the editor's Nuevo proyecto creates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NewProject {
    /// Template id or alias; empty = `basic`.
    pub template: String,
    /// `[game] name` (the template's own name when `None`).
    pub name: Option<String>,
    /// `[display]` width, height and scale (the template's when `None`).
    pub display: Option<(u32, u32, u32)>,
    /// Replaces `[palette]` and remaps every pixel-art sprite to it (`None` keeps the
    /// template's palette).
    pub palette: Option<Palette>,
}

/// `modengine new <folder>`: the basic template.
pub fn create(dir: &Path) -> Result<(), String> {
    create_with(dir, &NewProject::default())
}

/// Creates a project in `dir` (created if missing). Fails without writing anything if
/// `dir` already has a project or any of the template's files.
pub fn create_with(dir: &Path, opts: &NewProject) -> Result<(), String> {
    let id = if opts.template.trim().is_empty() { "basic" } else { opts.template.as_str() };
    let mut files = template_files(id).ok_or_else(|| format!("unknown template '{id}' (templates: {})", TEMPLATES.iter().map(|t| t.id).collect::<Vec<_>>().join(", ")))?;
    if dir.join("game.toml").exists() {
        return Err(format!("{} already contains a project", dir.display()));
    }
    if let Some((rel, _)) = files.iter().find(|(rel, _)| dir.join(rel).exists()) {
        return Err(format!("{} already exists", dir.join(rel).display()));
    }
    let gi = files.iter().position(|(r, _)| r == "game.toml").ok_or("template without game.toml")?;
    let mut game = String::from_utf8_lossy(&files[gi].1).to_string();
    if let Some(name) = &opts.name {
        game = set_key(&game, "game", "name", &quote(name));
        if has_section(&game, "android") {
            game = set_key(&game, "android", "package", &quote(&format!("com.modengine.{}", package_slug(name))));
        }
    }
    if let Some((w, h, s)) = opts.display {
        game = set_key(&game, "display", "width", &w.max(1).to_string());
        game = set_key(&game, "display", "height", &h.max(1).to_string());
        game = set_key(&game, "display", "scale", &s.max(1).to_string());
    }
    if let Some(pal) = &opts.palette {
        let old = palette_of(&game);
        let table = crate::palette::remap_table(&old, pal);
        game = replace_section(&game, "palette", &crate::palette::game_toml_section(pal));
        for (rel, bytes) in files.iter_mut() {
            if rel.starts_with("sprites/") && rel.ends_with(".toml") {
                let text = String::from_utf8_lossy(bytes).to_string();
                if let Some(new) = remap_sprite(&text, &table) {
                    *bytes = Cow::Owned(new.into_bytes());
                }
            }
        }
    }
    files[gi].1 = Cow::Owned(game.into_bytes());
    for (rel, content) in files {
        let p = dir.join(&rel);
        fs::create_dir_all(p.parent().unwrap()).map_err(|e| format!("{}: {e}", p.parent().unwrap().display()))?;
        fs::write(&p, &content).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    Ok(())
}

/// TOML basic string.
fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out += "\\\"",
            '\\' => out += "\\\\",
            '\n' => out += "\\n",
            c if (c as u32) < 0x20 => out += &format!("\\u{:04x}", c as u32),
            c => out.push(c),
        }
    }
    out + "\""
}

/// `My Game!` -> `mygame` (an Android package segment: ASCII letters/digits, not
/// starting with a digit).
pub fn package_slug(name: &str) -> String {
    let s: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect();
    match s.chars().next() {
        None => "game".into(),
        Some(c) if c.is_ascii_digit() => format!("game{s}"),
        _ => s,
    }
}

/// The table name of a `[section]` header line (`[[x]]` arrays are headers too).
fn header(line: &str) -> Option<&str> {
    let t = line.trim();
    let inner = t.strip_prefix('[')?;
    let inner = inner.strip_prefix('[').unwrap_or(inner);
    let end = inner.find(']')?;
    Some(inner[..end].trim())
}

fn has_section(text: &str, section: &str) -> bool {
    text.lines().any(|l| header(l) == Some(section))
}

/// Sets `key = value` (a TOML literal) in `[section]`, keeping every other line. Adds the
/// key (or the section) when missing.
pub fn set_key(text: &str, section: &str, key: &str, value: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let mut cur: Option<String> = None;
    let mut sec_at = None;
    for i in 0..lines.len() {
        if let Some(h) = header(&lines[i]) {
            cur = Some(h.to_string());
            if h == section && sec_at.is_none() {
                sec_at = Some(i);
            }
            continue;
        }
        if cur.as_deref() == Some(section) {
            let t = lines[i].trim_start();
            if let Some(rest) = t.strip_prefix(key) {
                if rest.trim_start().starts_with('=') {
                    lines[i] = format!("{key} = {value}");
                    return join(lines);
                }
            }
        }
    }
    match sec_at {
        Some(i) => lines.insert(i + 1, format!("{key} = {value}")),
        None => {
            lines.push(String::new());
            lines.push(format!("[{section}]"));
            lines.push(format!("{key} = {value}"));
        }
    }
    join(lines)
}

fn join(lines: Vec<String>) -> String {
    let mut s = lines.join("\n");
    s.push('\n');
    s
}

/// Replaces `[section]` (and the comment lines right above it) with `new` (a full
/// section text with its header); appends it when missing.
pub fn replace_section(text: &str, section: &str, new: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let Some(start) = lines.iter().position(|l| header(l) == Some(section)) else {
        return format!("{}\n\n{}", text.trim_end(), new);
    };
    let end = lines[start + 1..].iter().position(|l| header(l).is_some()).map(|p| start + 1 + p).unwrap_or(lines.len());
    let mut from = start;
    while from > 0 && lines[from - 1].trim_start().starts_with('#') {
        from -= 1;
    }
    let mut out: Vec<String> = lines[..from].iter().map(|s| s.to_string()).collect();
    out.extend(new.trim_end().lines().map(String::from));
    if end < lines.len() {
        out.push(String::new());
        out.extend(lines[end..].iter().map(|s| s.to_string()));
    }
    join(out)
}

/// The `[palette]` of a game.toml as characters and colours.
fn palette_of(game: &str) -> BTreeMap<char, u32> {
    let Ok(t) = game.parse::<toml::Table>() else { return BTreeMap::new() };
    t.get("palette")
        .and_then(|p| p.as_table())
        .map(|p| p.iter().filter_map(|(k, v)| Some((k.chars().next()?, crate::draw::parse_color(v.as_str()?)?))).collect())
        .unwrap_or_default()
}

/// Remaps the `'''` pixel blocks of a sprite. Sprites with their own `palette` are
/// left alone (their characters don't mean the project's colours).
fn remap_sprite(text: &str, table: &BTreeMap<char, char>) -> Option<String> {
    if !text.contains("pixels") || text.lines().any(|l| l.trim_start().starts_with("palette")) {
        return None;
    }
    let parts: Vec<&str> = text.split("'''").collect();
    let out: Vec<String> = parts.iter().enumerate().map(|(i, p)| if i % 2 == 1 { crate::palette::remap_rows(p, table) } else { p.to_string() }).collect();
    Some(out.join("'''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_line_edits_keep_the_rest() {
        let t = "# head\n[game]\nname = \"A\" # c\nstart_room = \"x\"\n\n# Paleta\n[palette]\n\"k\" = \"#000000\"\n\n[[layer]]\nx = 1\n";
        let t2 = set_key(t, "game", "name", &quote("Mi \"juego\""));
        assert!(t2.contains("name = \"Mi \\\"juego\\\"\"\nstart_room"), "{t2}");
        assert!(t2.starts_with("# head\n"));
        let t3 = set_key(&t2, "display", "width", "640");
        assert!(t3.trim_end().ends_with("[display]\nwidth = 640"), "{t3}");
        let t4 = replace_section(t, "palette", "[palette]\n\"0\" = \"#ffffff\"\n");
        assert!(!t4.contains("# Paleta") && !t4.contains("#000000") && t4.contains("\"0\" = \"#ffffff\"\n\n[[layer]]"), "{t4}");
        assert_eq!(package_slug("Mi Juego 2!"), "mijuego2");
        assert_eq!(package_slug("3D"), "game3d");
        assert_eq!(template("Plataformas").unwrap().id, "platformer");
        assert_eq!(template("mazmorra").unwrap().id, "crawler");
        assert!(template("nope").is_none());
    }

    #[test]
    fn every_template_has_files() {
        for t in TEMPLATES {
            let f = template_files(t.id).unwrap();
            assert!(f.iter().any(|(r, _)| r == "game.toml"), "{}", t.id);
            assert!(!f.iter().any(|(r, _)| r.starts_with("saves/")), "{}", t.id);
        }
    }
}

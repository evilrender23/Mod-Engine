//! Where the Agente panel's attachments come from: project assets (selected, focused, dragged
//! or `@`-mentioned), project files, the room selection and the Salida log. The app gathers
//! [`Sources`] every frame; the panel turns them into [`Attachment`]s.

use crate::agent::{AttachKind, Attachment};
use crate::app::{LogKind, LogLine, Open};
use crate::project::Project;
use std::path::{Path, PathBuf};

/// Lines of Salida the "Salida" attachment takes.
pub const OUTPUT_LINES: usize = 50;

/// What the panel can attach from the rest of the editor this frame.
#[derive(Default)]
pub struct Sources<'a> {
    pub project: Option<&'a Project>,
    /// The asset selected in the browser.
    pub selected: Option<Open>,
    /// The asset of the focused editor window.
    pub focused: Option<Open>,
    /// `(room name, the selected instance as the room file writes it)`.
    pub room_selection: Option<(String, String)>,
    pub log: &'a [LogLine],
}

impl Sources<'_> {
    pub fn asset(&self, o: &Open) -> Option<Attachment> {
        asset(self.project?, o)
    }

    pub fn output(&self) -> Option<Attachment> {
        output(self.log, OUTPUT_LINES)
    }

    pub fn room_selection(&self) -> Option<Attachment> {
        self.room_selection.as_ref().map(|(room, text)| room_selection(room, text))
    }
}

pub fn icon(o: &Open) -> &'static str {
    match o {
        Open::Room(_) => "▦",
        Open::Object(_) => "◆",
        Open::Sprite(_) => "▣",
        Open::Sound(_) => "♪",
        Open::Script(_) => "✎",
        Open::Font(_) => "🗛",
        Open::Note(_) => "🗒",
        Open::TileSet(_) => "▩",
        Open::Data(_) => "▤",
        Open::Settings => "⚙",
        Open::Anim(k, _) => k.icon(),
        Open::Fx(k, _) => k.icon(),
    }
}

/// `◆ Player`, for menus.
pub fn asset_label(o: &Open) -> String {
    match o {
        Open::Room(n) | Open::Object(n) | Open::Sprite(n) | Open::Sound(n) | Open::Script(n) => format!("{} {n}", icon(o)),
        Open::Font(n) | Open::Note(n) | Open::TileSet(n) | Open::Data(n) => format!("{} {n}", icon(o)),
        Open::Settings => "⚙ Ajustes".into(),
        Open::Anim(_, n) => format!("{} {n}", icon(o)),
        Open::Fx(_, n) => format!("{} {n}", icon(o)),
    }
}

/// The file that holds an asset, named for the agent: an object is the script that defines it,
/// the settings are `game.toml`, a PNG sprite is a picture.
pub fn asset(p: &Project, o: &Open) -> Option<Attachment> {
    let (path, label, about): (PathBuf, String, String) = match o {
        Open::Room(n) => (p.rooms.iter().find(|r| &r.name == n)?.path.clone(), n.clone(), format!("the room `{n}`")),
        Open::Object(n) => {
            let e = p.object(n)?;
            (p.scripts.get(e.script)?.path.clone(), n.clone(), format!("defines the object `{n}`"))
        }
        Open::Sprite(n) => (p.sprite_entry(n)?.path.clone(), n.clone(), format!("the sprite `{n}`")),
        Open::Sound(n) => (p.sounds.iter().find(|s| &s.name == n)?.path.clone(), n.clone(), format!("the sound `{n}`")),
        Open::Script(rel) => (p.dir.join(rel), rel.clone(), String::new()),
        Open::Settings => (p.dir.join("game.toml"), "game.toml".into(), "the game settings".into()),
        Open::Anim(k, n) => (p.motion.path(*k, n)?, n.clone(), format!("the {} `{n}` ({})", k.key(), k.folder())),
        Open::Fx(k, n) => (p.fx.path(*k, n)?, n.clone(), format!("the {} `{n}` ({})", k.key(), k.folder())),
        Open::Font(n) => (p.fonts.iter().find(|s| &s.name == n)?.path.clone(), n.clone(), format!("the font `{n}`")),
        Open::Note(n) => (p.notes.iter().find(|s| &s.name == n)?.path.clone(), n.clone(), format!("the note `{n}`")),
        Open::TileSet(n) => (p.tilesets.iter().find(|s| &s.name == n)?.path.clone(), n.clone(), format!("the tile set `{n}`")),
        Open::Data(n) => (p.data.iter().find(|s| &s.name == n)?.path.clone(), n.clone(), format!("the data table `{n}` (data_get(\"{n}\", id))")),
    };
    let mut a = Attachment::file(&path);
    a.label = label;
    a.about = about;
    if a.kind != AttachKind::Image {
        a.icon = icon(o);
    }
    Some(a)
}

/// The last `n` lines of Salida; `None` when it is empty.
pub fn output(log: &[LogLine], n: usize) -> Option<Attachment> {
    if log.is_empty() {
        return None;
    }
    let text: Vec<String> = log[log.len().saturating_sub(n)..]
        .iter()
        .map(|l| {
            let tag = match l.kind {
                LogKind::Info => "",
                LogKind::Ok => "[ok] ",
                LogKind::Error => "[error] ",
                LogKind::Game => "[juego] ",
            };
            if l.text.starts_with('[') {
                l.text.clone()
            } else {
                format!("{tag}{}", l.text)
            }
        })
        .collect();
    let mut a = Attachment::text("☰", "Salida", text.join("\n"));
    a.about = "the editor's output log: messages of the last check and the game".into();
    Some(a)
}

pub fn room_selection(room: &str, text: &str) -> Attachment {
    let mut a = Attachment::text("⬚", format!("Selección en {room}"), text);
    a.about = format!("the instance selected in the room editor, as `rooms/{room}.toml` writes it, including unsaved changes");
    a
}

/// Every asset of the project, for menus and `@` mentions.
pub fn all_assets(p: &Project) -> Vec<Open> {
    let mut v: Vec<Open> = vec![];
    v.extend(p.rooms.iter().map(|r| Open::Room(r.name.clone())));
    v.extend(p.objects.iter().map(|o| Open::Object(o.name.clone())));
    v.extend(p.sprites.iter().map(|s| Open::Sprite(s.name.clone())));
    v.extend(p.sounds.iter().map(|s| Open::Sound(s.name.clone())));
    v.extend(p.scripts.iter().filter(|s| s.rel.starts_with("scripts/")).map(|s| Open::Script(s.rel.clone())));
    v.push(Open::Settings);
    v
}

/// Files of the project, relative and sorted, without hidden folders (`.trash`, `.git`) or
/// build output; at most `max`.
pub fn project_files(dir: &Path, max: usize) -> Vec<String> {
    fn walk(root: &Path, d: &Path, out: &mut Vec<String>, max: usize, depth: u32) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            if out.len() >= max {
                return;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "target" {
                continue;
            }
            let p = e.path();
            if p.is_dir() {
                if depth < 8 {
                    walk(root, &p, out, max, depth + 1);
                }
            } else if let Ok(rel) = p.strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = vec![];
    walk(dir, dir, &mut out, max, 0);
    out
}

/// Case-insensitive "all words appear" match, for the filters.
pub fn matches(text: &str, query: &str) -> bool {
    let t = text.to_lowercase();
    query.to_lowercase().split_whitespace().all(|w| t.contains(w))
}

/// The `@word` being typed at the end of `text`: its byte offset and the word after `@`.
pub fn mention_at_end(text: &str) -> Option<(usize, &str)> {
    let start = text.rfind(|c: char| c.is_whitespace()).map(|i| i + text[i..].chars().next().map_or(1, char::len_utf8)).unwrap_or(0);
    let word = &text[start..];
    word.strip_prefix('@').filter(|w| !w.contains('@')).map(|w| (start, w))
}

/// A path typed by the user: `~/x`, absolute, or relative to the project.
pub fn resolve_path(root: Option<&Path>, typed: &str) -> PathBuf {
    let t = typed.trim().trim_matches(|c| c == '"' || c == '\'');
    if let Some(rest) = t.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    let p = PathBuf::from(t);
    match root {
        Some(r) if p.is_relative() => r.join(p),
        _ => p,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assets_become_their_files() {
        let dir = crate::room::tests::temp_project("attach").canonicalize().unwrap();
        let p = Project::open(&dir).unwrap();
        let obj = p.objects[0].name.clone();
        let a = asset(&p, &Open::Object(obj.clone())).unwrap();
        assert_eq!((a.kind, a.icon, a.label.as_str()), (AttachKind::File, "◆", obj.as_str()));
        assert_eq!(a.path, p.scripts[p.objects[0].script].path.canonicalize().unwrap());
        assert!(a.about.contains(&obj));
        let room = p.rooms[0].name.clone();
        assert_eq!(asset(&p, &Open::Room(room.clone())).unwrap().path, dir.join(format!("rooms/{room}.toml")));
        assert_eq!(asset(&p, &Open::Settings).unwrap().path, dir.join("game.toml"));
        assert!(asset(&p, &Open::Room("nope".into())).is_none());
        // The file list skips hidden folders.
        std::fs::create_dir_all(dir.join(".trash")).unwrap();
        std::fs::write(dir.join(".trash/old.ms"), "").unwrap();
        let files = project_files(&dir, 1000);
        assert!(files.contains(&"game.toml".to_string()) && files.iter().any(|f| f.starts_with("rooms/")));
        assert!(!files.iter().any(|f| f.contains(".trash")));
        assert!(all_assets(&p).contains(&Open::Object(obj)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn output_takes_the_last_lines() {
        assert!(output(&[], 50).is_none());
        let log: Vec<LogLine> = (0..80).map(|i| LogLine { kind: if i % 2 == 0 { LogKind::Error } else { LogKind::Info }, text: format!("l{i}") }).collect();
        let a = output(&log, 50).unwrap();
        assert_eq!(a.kind, AttachKind::Text);
        assert!(a.text.starts_with("[error] l30\nl31\n") && a.text.ends_with("l79"), "{}", a.text);
    }

    #[test]
    fn mentions_and_paths() {
        assert_eq!(mention_at_end("mira @pla"), Some((5, "pla")));
        assert_eq!(mention_at_end("@"), Some((0, "")));
        assert_eq!(mention_at_end("año\n@é"), Some((5, "é")));
        assert_eq!(mention_at_end("mira @pla "), None);
        assert_eq!(mention_at_end("correo a@b"), None);
        assert!(matches("objects/Player.ms", "play ms"));
        assert!(!matches("objects/Player.ms", "enemy"));
        assert_eq!(resolve_path(Some(Path::new("/p")), "rooms/a.toml"), PathBuf::from("/p/rooms/a.toml"));
        assert_eq!(resolve_path(Some(Path::new("/p")), "'/x/y.png'"), PathBuf::from("/x/y.png"));
    }
}

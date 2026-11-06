//! After an agent turn: what it touched, as editor windows. The files the turn added or changed
//! are mapped back to project assets (a sprite's TOML or PNG, a room, the script that defines an
//! object…) and the app opens them in a new workspace, laid out in a grid by kind.

use crate::app::Open;
use crate::motion::AnimKind;
use crate::project::Project;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Most windows a turn opens; a big import only shows the first ones.
pub const MAX_WINDOWS: usize = 12;

/// Every asset of the project with the files that hold it.
fn asset_files(p: &Project) -> Vec<(Open, Vec<PathBuf>)> {
    let mut v: Vec<(Open, Vec<PathBuf>)> = vec![];
    for r in &p.rooms {
        v.push((Open::Room(r.name.clone()), vec![r.path.clone()]));
    }
    for o in &p.objects {
        if let Some(s) = p.scripts.get(o.script) {
            v.push((Open::Object(o.name.clone()), vec![s.path.clone()]));
        }
    }
    for s in &p.sprites {
        // `hero.toml` may draw `hero.png`: either file is the sprite.
        v.push((Open::Sprite(s.name.clone()), vec![s.path.clone(), s.path.with_extension("png"), s.path.with_extension("toml")]));
    }
    for s in &p.sounds {
        v.push((Open::Sound(s.name.clone()), vec![s.path.clone(), s.path.with_extension("toml")]));
    }
    for f in &p.fonts {
        v.push((Open::Font(f.name.clone()), vec![f.path.clone(), f.path.with_extension("toml")]));
    }
    for t in &p.tilesets {
        v.push((Open::TileSet(t.name.clone()), vec![t.path.clone()]));
    }
    for n in &p.notes {
        v.push((Open::Note(n.name.clone()), vec![n.path.clone()]));
    }
    for k in AnimKind::ALL {
        for n in p.motion.names(k) {
            if let Some(path) = p.motion.path(k, &n) {
                v.push((Open::Anim(k, n), vec![path]));
            }
        }
    }
    for k in [crate::fx::FxKind::Particles, crate::fx::FxKind::Shader, crate::fx::FxKind::Extension] {
        for n in p.fx.names(k) {
            if let Some(path) = p.fx.path(k, &n) {
                // Shaders are a .fsh/.vsh pair; extensions a folder.
                let mut files = vec![path.clone(), path.with_extension("vsh"), path.with_extension("fsh")];
                if let Some(d) = path.parent() {
                    files.push(d.to_path_buf());
                }
                v.push((Open::Fx(k, n), files));
            }
        }
    }
    // Scripts that do not define objects (objects/ scripts are opened as their objects).
    for s in &p.scripts {
        if s.rel.starts_with("scripts/") {
            v.push((Open::Script(s.rel.clone()), vec![s.path.clone()]));
        }
    }
    v.push((Open::Settings, vec![p.dir.join("game.toml")]));
    v
}

/// Whether a changed line falls in the object's definition (always, when its script only
/// defines that object).
fn object_changed(p: &Project, name: &str, changed: &[usize]) -> bool {
    let Some(script) = p.object(name).and_then(|o| p.scripts.get(o.script)) else { return true };
    let found = crate::objsrc::find_objects(&script.text);
    if found.len() <= 1 {
        return true;
    }
    let Some(f) = found.iter().find(|f| f.name == name) else { return true };
    let line_of = |byte: usize| script.text[..byte.min(script.text.len())].matches('\n').count() + 1;
    let (a, b) = (line_of(f.span.start), line_of(f.span.end));
    changed.iter().any(|l| (a..=b).contains(l))
}

fn same(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

/// Assets behind `rels` (project-relative paths the turn added or modified), in a stable order:
/// rooms, objects, sprites…, each kind sorted by name. Files that are no asset (CREDITS.md,
/// notes of the agent) are skipped; an extension's folder matches any file inside it.
/// `lines` narrows a modified script that defines several objects to the objects whose
/// definition holds a changed line (see [`crate::agent_panel::TurnResult::lines`]).
pub fn assets_for(p: &Project, rels: &[String], lines: &BTreeMap<String, Vec<usize>>) -> Vec<Open> {
    let files: Vec<(PathBuf, &String)> = rels.iter().map(|r| (p.dir.join(r), r)).collect();
    let mut out: Vec<Open> = vec![];
    for (open, paths) in asset_files(p) {
        let Some((_, rel)) = files.iter().find(|(f, _)| paths.iter().any(|a| same(f, a) || (a.is_dir() && f.starts_with(a)))) else { continue };
        if let (Open::Object(name), Some(changed)) = (&open, lines.get(*rel)) {
            if !object_changed(p, name, changed) {
                continue;
            }
        }
        if !out.contains(&open) {
            out.push(open);
        }
    }
    out.sort_by_key(|o| (rank(o), crate::agent_attach::asset_label(o)));
    out
}

fn rank(o: &Open) -> u8 {
    match o {
        Open::Room(_) => 0,
        Open::Object(_) => 1,
        Open::Sprite(_) => 2,
        Open::TileSet(_) => 3,
        Open::Anim(..) => 4,
        Open::Fx(..) => 5,
        Open::Sound(_) => 6,
        Open::Font(_) => 7,
        Open::Script(_) => 8,
        Open::Note(_) => 9,
        Open::Data(_) => 9,
        Open::Settings => 10,
    }
}

/// Name of the workspace for a turn: "Agente: <first words of the request>".
pub fn workspace_name(prompt: &str) -> String {
    let words: Vec<&str> = prompt.split_whitespace().take(4).collect();
    let mut t = words.join(" ");
    if t.chars().count() > 28 {
        t = t.chars().take(27).collect::<String>() + "…";
    } else if prompt.split_whitespace().count() > 4 {
        t.push('…');
    }
    if t.is_empty() {
        "Agente".into()
    } else {
        format!("Agente: {t}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_changed_files_to_assets() {
        let dir = crate::room::tests::temp_project("agent_ws");
        let p = Project::open(&dir).unwrap();
        let room = p.rooms[0].name.clone();
        let obj = p.objects[0].name.clone();
        let obj_rel = p.scripts[p.objects[0].script].rel.clone();
        let spr = p.sprites[0].name.clone();
        let spr_rel = p.sprites[0].path.strip_prefix(&p.dir).unwrap().to_string_lossy().replace('\\', "/");
        let rels = vec![
            "CREDITS.md".to_string(),
            spr_rel,
            format!("rooms/{room}.toml"),
            obj_rel,
            "game.toml".into(),
            "no/existe.png".into(),
        ];
        let got = assets_for(&p, &rels, &BTreeMap::new());
        // a new-to-the-turn script opens every object it defines (here, several)
        let others: Vec<&Open> = got.iter().filter(|o| !matches!(o, Open::Object(_))).collect();
        assert_eq!(others, [&Open::Room(room), &Open::Sprite(spr), &Open::Settings]);
        assert!(got.contains(&Open::Object(obj)));
        assert!(matches!(got[0], Open::Room(_)) && matches!(got[1], Open::Object(_)) && got.last() == Some(&Open::Settings), "rooms, objects… settings: {got:?}");
        assert!(assets_for(&p, &["CREDITS.md".into()], &BTreeMap::new()).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_shared_script_only_opens_the_objects_that_changed() {
        let dir = crate::room::tests::temp_project("agent_ws_obj");
        let rel = "objects/actors.ms";
        let old = std::fs::read_to_string(dir.join(rel)).unwrap();
        let p = Project::open(&dir).unwrap();
        let shared: Vec<String> = crate::objsrc::find_objects(&old).into_iter().map(|f| f.name).collect();
        assert!(shared.len() > 2, "{shared:?}");
        // change one line inside the second object
        let second = &crate::objsrc::find_objects(&old)[1];
        let line = old[..second.span.start].matches('\n').count() + 2;
        let mut new_lines: Vec<&str> = old.lines().collect();
        new_lines.insert(line - 1, "    // tuned by the agent");
        let new = new_lines.join("\n") + "\n";
        let changed = crate::agent_panel::changed_lines(&old, &new);
        assert_eq!(changed, [line]);
        let lines = BTreeMap::from([(rel.to_string(), changed)]);
        assert_eq!(assets_for(&p, &[rel.into()], &lines), [Open::Object(shared[1].clone())]);
        // a new file (no lines) opens every object it defines
        assert_eq!(assets_for(&p, &[rel.into()], &BTreeMap::new()).len(), shared.len());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_workspaces_after_the_request() {
        assert_eq!(workspace_name("añade un enemigo que dispara"), "Agente: añade un enemigo que…");
        assert_eq!(workspace_name("haz un slime"), "Agente: haz un slime");
        assert_eq!(workspace_name("  "), "Agente");
        assert!(workspace_name("supercalifragilisticoespialidoso extraordinariamente largo").chars().count() <= 36);
    }
}

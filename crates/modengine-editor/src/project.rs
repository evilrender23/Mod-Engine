//! The project as the editor sees it: every asset file, loaded leniently so one
//! broken file never prevents editing the rest.

use crate::objsrc::{self, ObjectSrc};
use modengine::project::{asset_files, collect_scripts, load_sprite, GameToml, Sprite};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub struct SpriteEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_toml: bool,
    pub data: Result<Sprite, String>,
    /// Bumped on every reload so textures know when to refresh.
    pub gen: u64,
}

pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_toml: bool,
}

pub struct Script {
    pub rel: String,
    pub path: PathBuf,
    pub text: String,
    pub saved: String,
    /// Bumped whenever `text` changes (object editors watch it).
    pub rev: u64,
}

impl Script {
    pub fn dirty(&self) -> bool {
        self.text != self.saved
    }
}

/// A tile set file and its tiles, cut with the engine's own loader.
pub struct TileSetEntry {
    pub name: String,
    pub path: PathBuf,
    pub data: Result<modengine::tileset::TileSetAsset, String>,
    /// Bumped on every reload so textures know when to refresh.
    pub gen: u64,
}

/// A prefab file (`prefabs/<name>.toml`): a group of instances placed together.
pub struct PrefabEntry {
    pub name: String,
    pub path: PathBuf,
    pub data: Result<modengine::prefab::PrefabToml, String>,
}

pub struct ObjEntry {
    pub name: String,
    pub parent: Option<String>,
    pub script: usize,
    pub model: Option<ObjectSrc>,
}

pub struct Project {
    pub dir: PathBuf,
    pub game: GameToml,
    pub game_error: Option<String>,
    pub sprites: Vec<SpriteEntry>,
    pub sounds: Vec<FileEntry>,
    pub fonts: Vec<FileEntry>,
    /// `notes/*.md`: notes for the team, not used by the game.
    pub notes: Vec<FileEntry>,
    pub tilesets: Vec<TileSetEntry>,
    pub rooms: Vec<FileEntry>,
    /// `prefabs/*.toml` (prefab.rs).
    pub prefabs: Vec<PrefabEntry>,
    /// `data/*.toml|json|csv` tables (datatable.rs); schemas are not listed.
    pub data: Vec<FileEntry>,
    /// paths/, timelines/, curves/ and sequences/ (motion.rs).
    pub motion: crate::motion::Lists,
    /// particles/, shaders/ and extensions/ (fx.rs).
    pub fx: crate::fx::Lists,
    pub scripts: Vec<Script>,
    pub objects: Vec<ObjEntry>,
    gen: u64,
    /// game.toml as last read, to notice when something else changes it.
    game_text: String,
}

/// What [`Project::sync_from_disk`] found.
#[derive(Default, Debug, PartialEq)]
pub struct Synced {
    /// Scripts reloaded, added or removed (relative paths).
    pub scripts: Vec<String>,
    /// Scripts changed on disk while they had unsaved edits here: the edits are kept.
    pub conflicts: Vec<String>,
    /// game.toml changed and was read again.
    pub game: bool,
}

fn entries(dir: &Path, exts: &[&str]) -> Vec<FileEntry> {
    asset_files(dir, exts).into_iter().map(|(name, path, is_toml)| FileEntry { name, path, is_toml }).collect()
}

fn prefab_entries(dir: &Path) -> Vec<PrefabEntry> {
    asset_files(&dir.join("prefabs"), &[])
        .into_iter()
        .filter(|(_, _, is_toml)| *is_toml)
        .map(|(name, path, _)| {
            let data = fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| toml::from_str(&t).map_err(|e| e.to_string()));
            PrefabEntry { name, path, data }
        })
        .collect()
}

fn data_entries(dir: &Path) -> Vec<FileEntry> {
    modengine::datatable::table_files(dir).into_iter().map(|(name, path, f)| FileEntry { name, path, is_toml: f == modengine::datatable::Format::Toml }).collect()
}

impl Project {
    pub fn open(dir: &Path) -> Result<Project, String> {
        let game_path = dir.join("game.toml");
        let text = fs::read_to_string(&game_path).map_err(|e| format!("{}: {e}", game_path.display()))?;
        let (game, game_error) = match toml::from_str::<GameToml>(&text) {
            Ok(g) => (g, None),
            Err(e) => (GameToml::default(), Some(e.to_string())),
        };
        let mut p = Project {
            dir: dir.to_path_buf(),
            game,
            game_error,
            sprites: vec![],
            sounds: entries(&dir.join("sounds"), &["wav", "ogg"]),
            fonts: entries(&dir.join("fonts"), &["ttf", "otf"]),
            notes: entries(&dir.join("notes"), &["md"]).into_iter().filter(|e| !e.is_toml).collect(),
            tilesets: vec![],
            rooms: entries(&dir.join("rooms"), &[]),
            prefabs: prefab_entries(dir),
            data: data_entries(dir),
            motion: crate::motion::Lists::load(dir),
            fx: crate::fx::Lists::load(dir),
            scripts: vec![],
            objects: vec![],
            gen: 0,
            game_text: text,
        };
        p.reload_sprites();
        let mut raw = Vec::new();
        collect_scripts(dir, dir, &mut raw)?;
        raw.sort();
        p.scripts = raw
            .into_iter()
            .map(|(rel, text)| Script { path: dir.join(&rel), rel, saved: text.clone(), text, rev: 0 })
            .collect();
        p.rescan_objects();
        Ok(p)
    }

    /// Picks up changes another program (the Agente panel's CLI, git, a text editor) made to the
    /// scripts and game.toml. Scripts with unsaved edits keep them; their saved copy becomes the
    /// file's, so saving overwrites it and reverting gets it. Lists and sprites are reloaded by
    /// [`Project::reload_lists`].
    pub fn sync_from_disk(&mut self) -> Synced {
        let mut out = Synced::default();
        let mut raw = Vec::new();
        let dir = self.dir.clone();
        if collect_scripts(&dir, &dir, &mut raw).is_err() {
            return out;
        }
        raw.sort();
        let on_disk: BTreeSet<String> = raw.iter().map(|(r, _)| r.clone()).collect();
        for (rel, text) in raw {
            match self.script_index(&rel) {
                Some(i) if self.scripts[i].saved == text => {}
                Some(i) => {
                    let s = &mut self.scripts[i];
                    if s.dirty() && s.text != text {
                        out.conflicts.push(rel);
                        s.saved = text;
                    } else {
                        s.saved = text.clone();
                        s.text = text;
                        s.rev += 1;
                        out.scripts.push(rel);
                    }
                }
                None => {
                    self.scripts.push(Script { path: dir.join(&rel), rel: rel.clone(), saved: text.clone(), text, rev: 1 });
                    out.scripts.push(rel);
                }
            }
        }
        // Deleted on disk (unless they have unsaved edits, such as a script not saved yet).
        let before = self.scripts.len();
        let mut gone = vec![];
        self.scripts.retain(|s| {
            let keep = on_disk.contains(&s.rel) || s.dirty();
            if !keep {
                gone.push(s.rel.clone());
            }
            keep
        });
        out.scripts.extend(gone);
        if !out.scripts.is_empty() || self.scripts.len() != before {
            self.scripts.sort_by(|a, b| a.rel.cmp(&b.rel));
            self.rescan_objects();
        }
        if let Ok(text) = fs::read_to_string(dir.join("game.toml")) {
            if text != self.game_text {
                match toml::from_str::<GameToml>(&text) {
                    Ok(g) => {
                        self.game = g;
                        self.game_error = None;
                    }
                    Err(e) => self.game_error = Some(e.to_string()),
                }
                self.game_text = text;
                out.game = true;
            }
        }
        out
    }

    pub fn reload_sprites(&mut self) {
        let palette = self.game.palette.clone();
        self.sprites = asset_files(&self.dir.join("sprites"), &["png"])
            .into_iter()
            .map(|(name, path, is_toml)| {
                self.gen += 1;
                let data = load_sprite(&name, &path, is_toml, &palette);
                SpriteEntry { name, path, is_toml, data, gen: self.gen }
            })
            .collect();
        self.reload_tilesets();
    }

    /// Tile sets are cut from sprites: reloaded whenever the sprites are.
    pub fn reload_tilesets(&mut self) {
        let files = asset_files(&self.dir.join("tilesets"), &[]);
        let mut out = Vec::new();
        for (name, path, _) in files {
            self.gen += 1;
            let data = modengine::tileset::load_tileset(&name, &path, &|n| self.sprite(n));
            out.push(TileSetEntry { name, path, data, gen: self.gen });
        }
        self.tilesets = out;
    }

    pub fn tileset(&self, name: &str) -> Option<&modengine::tileset::TileSetAsset> {
        self.tilesets.iter().find(|t| t.name == name).and_then(|t| t.data.as_ref().ok())
    }

    pub fn reload_lists(&mut self) {
        self.sounds = entries(&self.dir.join("sounds"), &["wav", "ogg"]);
        self.fonts = entries(&self.dir.join("fonts"), &["ttf", "otf"]);
        self.notes = entries(&self.dir.join("notes"), &["md"]).into_iter().filter(|e| !e.is_toml).collect();
        self.rooms = entries(&self.dir.join("rooms"), &[]);
        self.prefabs = prefab_entries(&self.dir);
        self.data = data_entries(&self.dir);
        self.motion = crate::motion::Lists::load(&self.dir);
        self.fx = crate::fx::Lists::load(&self.dir);
        self.reload_sprites();
    }

    pub fn rescan_objects(&mut self) {
        let mut objs = Vec::new();
        for (si, s) in self.scripts.iter().enumerate() {
            for f in objsrc::find_objects(&s.text) {
                objs.push(ObjEntry { name: f.name, parent: f.parent, script: si, model: f.model });
            }
        }
        objs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        self.objects = objs;
    }

    pub fn set_script_text(&mut self, idx: usize, text: String) {
        let s = &mut self.scripts[idx];
        if s.text != text {
            s.text = text;
            s.rev += 1;
            self.rescan_objects();
        }
    }

    pub fn save_script(&mut self, idx: usize) -> Result<(), String> {
        let s = &mut self.scripts[idx];
        if let Some(parent) = s.path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(&s.path, &s.text).map_err(|e| format!("{}: {e}", s.path.display()))?;
        s.saved = s.text.clone();
        Ok(())
    }

    pub fn object(&self, name: &str) -> Option<&ObjEntry> {
        self.objects.iter().find(|o| o.name == name)
    }

    pub fn object_names(&self) -> Vec<String> {
        self.objects.iter().map(|o| o.name.clone()).collect()
    }

    /// Sprite drawn for an object: its own `sprite = ...` or the closest parent's.
    pub fn object_sprite(&self, name: &str) -> Option<String> {
        let mut cur = Some(name.to_string());
        let mut guard = 0;
        while let Some(n) = cur {
            guard += 1;
            if guard > 32 {
                return None;
            }
            let o = self.object(&n)?;
            if let Some(v) = o.model.as_ref().and_then(|m| m.prop("sprite")) {
                let v = v.split("//").next().unwrap_or("").trim();
                return self.sprite(v).map(|_| v.to_string());
            }
            cur = o.parent.clone();
        }
        None
    }

    pub fn sprite_entry(&self, name: &str) -> Option<&SpriteEntry> {
        self.sprites.iter().find(|s| s.name == name)
    }

    pub fn sprite(&self, name: &str) -> Option<&Sprite> {
        self.sprite_entry(name).and_then(|s| s.data.as_ref().ok())
    }

    pub fn sprite_names(&self) -> Vec<String> {
        self.sprites.iter().map(|s| s.name.clone()).collect()
    }

    pub fn room_names(&self) -> Vec<String> {
        self.rooms.iter().map(|r| r.name.clone()).collect()
    }

    pub fn prefab_names(&self) -> Vec<String> {
        self.prefabs.iter().map(|r| r.name.clone()).collect()
    }

    pub fn prefab(&self, name: &str) -> Option<&modengine::prefab::PrefabToml> {
        self.prefabs.iter().find(|p| p.name == name).and_then(|p| p.data.as_ref().ok())
    }

    pub fn is_prefab(&self, name: &str) -> bool {
        self.prefabs.iter().any(|p| p.name == name)
    }

    /// The plain object instances of a prefab, nested prefabs expanded (relative to its origin).
    pub fn prefab_items(&self, name: &str) -> Result<Vec<modengine::prefab::FlatItem>, String> {
        modengine::prefab::flatten(name, &|n| self.prefab(n))
    }

    /// Every name already taken (assets share one namespace).
    pub fn taken_names(&self) -> BTreeSet<String> {
        let mut s = BTreeSet::new();
        s.extend(self.sprites.iter().map(|x| x.name.clone()));
        s.extend(self.sounds.iter().map(|x| x.name.clone()));
        s.extend(self.fonts.iter().map(|x| x.name.clone()));
        s.extend(self.notes.iter().map(|x| x.name.clone()));
        s.extend(self.tilesets.iter().map(|x| x.name.clone()));
        s.extend(self.rooms.iter().map(|x| x.name.clone()));
        s.extend(self.prefabs.iter().map(|x| x.name.clone()));
        s.extend(self.objects.iter().map(|x| x.name.clone()));
        s.extend(self.motion.all_names());
        s.extend(self.fx.asset_names());
        s
    }

    pub fn dirty_scripts(&self) -> bool {
        self.scripts.iter().any(|s| s.dirty())
    }

    pub fn script_index(&self, rel: &str) -> Option<usize> {
        self.scripts.iter().position(|s| s.rel == rel)
    }

    /// Adds a new script file in memory (saved with the rest).
    pub fn add_script(&mut self, rel: &str, text: String) -> usize {
        self.scripts.push(Script { rel: rel.to_string(), path: self.dir.join(rel), text, saved: String::new(), rev: 1 });
        self.rescan_objects();
        self.scripts.len() - 1
    }

    /// Moves a file into `<project>/.trash/` instead of deleting it.
    pub fn trash(&self, path: &Path) -> Result<(), String> {
        let trash = self.dir.join(".trash");
        fs::create_dir_all(&trash).map_err(|e| e.to_string())?;
        let name = path.file_name().ok_or("bad path")?;
        let mut target = trash.join(name);
        let mut n = 1;
        while target.exists() {
            target = trash.join(format!("{}.{n}", name.to_string_lossy()));
            n += 1;
        }
        fs::rename(path, &target).map_err(|e| format!("{}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_from_disk_keeps_unsaved_edits() {
        let dir = crate::room::tests::temp_project("sync");
        let mut p = Project::open(&dir).unwrap();
        let coin = p.script_index("objects/world.ms").unwrap();
        let other = (0..p.scripts.len()).find(|&i| i != coin).unwrap();
        let other_rel = p.scripts[other].rel.clone();
        // The user is editing one script; the agent changes both, adds one and edits game.toml.
        let mine = format!("{}// mine\n", p.scripts[other].text);
        p.set_script_text(other, mine.clone());
        let coin_new = format!("{}// agent\n", p.scripts[coin].text);
        fs::write(dir.join("objects/world.ms"), &coin_new).unwrap();
        fs::write(dir.join(&other_rel), "// agent rewrote it\n").unwrap();
        fs::write(dir.join("objects/Bat.ms"), "object Bat {\n}\n").unwrap();
        let game = fs::read_to_string(dir.join("game.toml")).unwrap().replace("Coin Climber", "Bat Climber");
        fs::write(dir.join("game.toml"), game).unwrap();

        let s = p.sync_from_disk();
        assert_eq!(s.conflicts, vec![other_rel.clone()]);
        assert!(s.scripts.contains(&"objects/world.ms".to_string()) && s.scripts.contains(&"objects/Bat.ms".to_string()));
        assert!(s.game);
        assert_eq!(p.game.game.name, "Bat Climber");
        let coin = p.script_index("objects/world.ms").unwrap();
        assert_eq!(p.scripts[coin].text, coin_new);
        assert!(!p.scripts[coin].dirty());
        let other = p.script_index(&other_rel).unwrap();
        assert_eq!(p.scripts[other].text, mine);
        assert_eq!(p.scripts[other].saved, "// agent rewrote it\n");
        assert!(p.object("Bat").is_some());

        // Nothing changed: nothing to report. A deleted file goes away.
        assert_eq!(p.sync_from_disk(), Synced::default());
        fs::remove_file(dir.join("objects/Bat.ms")).unwrap();
        assert_eq!(p.sync_from_disk().scripts, vec!["objects/Bat.ms".to_string()]);
        assert!(p.object("Bat").is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}

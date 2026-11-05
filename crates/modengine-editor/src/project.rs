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
    pub rooms: Vec<FileEntry>,
    pub scripts: Vec<Script>,
    pub objects: Vec<ObjEntry>,
    gen: u64,
}

fn entries(dir: &Path, exts: &[&str]) -> Vec<FileEntry> {
    asset_files(dir, exts).into_iter().map(|(name, path, is_toml)| FileEntry { name, path, is_toml }).collect()
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
            rooms: entries(&dir.join("rooms"), &[]),
            scripts: vec![],
            objects: vec![],
            gen: 0,
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
    }

    pub fn reload_lists(&mut self) {
        self.sounds = entries(&self.dir.join("sounds"), &["wav", "ogg"]);
        self.fonts = entries(&self.dir.join("fonts"), &["ttf", "otf"]);
        self.rooms = entries(&self.dir.join("rooms"), &[]);
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

    /// Every name already taken (assets share one namespace).
    pub fn taken_names(&self) -> BTreeSet<String> {
        let mut s = BTreeSet::new();
        s.extend(self.sprites.iter().map(|x| x.name.clone()));
        s.extend(self.sounds.iter().map(|x| x.name.clone()));
        s.extend(self.fonts.iter().map(|x| x.name.clone()));
        s.extend(self.rooms.iter().map(|x| x.name.clone()));
        s.extend(self.objects.iter().map(|x| x.name.clone()));
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

//! Where project files come from: a folder on disk or a game pack (`game.dat`).
//!
//! Loaders keep working with ordinary paths. A pack is *mounted* at a root path (normally
//! the pack file itself, e.g. `/opt/mygame/game.dat`); any path under that root is then
//! served from the pack, everything else from the disk. A project folder behaves exactly
//! as before: nothing is mounted for it.

use crate::pack::Pack;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};

/// A source of project files.
#[derive(Clone)]
pub enum Source {
    Dir(PathBuf),
    Pack { root: PathBuf, pack: Arc<Pack> },
}

impl Source {
    /// The path the project is loaded from (`Engine::load` / `Project::load`).
    pub fn root(&self) -> &Path {
        match self {
            Source::Dir(d) => d,
            Source::Pack { root, .. } => root,
        }
    }

    pub fn is_pack(&self) -> bool {
        matches!(self, Source::Pack { .. })
    }
}

static MOUNTS: RwLock<Vec<(PathBuf, Arc<Pack>)>> = RwLock::new(Vec::new());

/// Serves the pack's files under `root` from now on. Returns the source to load from.
pub fn mount(root: &Path, pack: Pack) -> Source {
    let pack = Arc::new(pack);
    let mut m = MOUNTS.write().unwrap();
    m.retain(|(r, _)| r != root);
    m.push((root.to_path_buf(), pack.clone()));
    Source::Pack { root: root.to_path_buf(), pack }
}

pub fn unmount(root: &Path) {
    MOUNTS.write().unwrap().retain(|(r, _)| r != root);
}

/// Is `path` (or one of its parents) a mounted pack?
pub fn is_mounted(path: &Path) -> bool {
    lookup(path).is_some()
}

/// The pack serving `path` and the path inside it ("" = root, `/` separated).
fn lookup(path: &Path) -> Option<(Arc<Pack>, String)> {
    let m = MOUNTS.read().unwrap();
    let (root, pack) = m.iter().filter(|(r, _)| path.starts_with(r)).max_by_key(|(r, _)| r.components().count())?;
    let mut parts: Vec<String> = vec![];
    for c in path.strip_prefix(root).ok()?.components() {
        match c {
            Component::Normal(s) => parts.push(s.to_string_lossy().to_string()),
            Component::ParentDir => {
                parts.pop();
            }
            _ => {}
        }
    }
    Some((pack.clone(), parts.join("/")))
}

fn not_found() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "not found in the game pack")
}

pub fn read(path: &Path) -> io::Result<Vec<u8>> {
    match lookup(path) {
        Some((pack, rel)) => pack.file(&rel).map(|b| b.to_vec()).ok_or_else(not_found),
        None => std::fs::read(path),
    }
}

pub fn read_to_string(path: &Path) -> io::Result<String> {
    match lookup(path) {
        Some(_) => String::from_utf8(read(path)?).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "stream did not contain valid UTF-8")),
        None => std::fs::read_to_string(path),
    }
}

pub fn is_file(path: &Path) -> bool {
    match lookup(path) {
        Some((pack, rel)) => pack.file(&rel).is_some(),
        None => path.is_file(),
    }
}

pub fn is_dir(path: &Path) -> bool {
    match lookup(path) {
        Some((pack, rel)) => pack.is_dir(&rel),
        None => path.is_dir(),
    }
}

pub fn exists(path: &Path) -> bool {
    is_file(path) || is_dir(path)
}

/// Entries of a folder: (full path, is_dir), in no particular order (callers sort).
pub fn read_dir(path: &Path) -> io::Result<Vec<(PathBuf, bool)>> {
    match lookup(path) {
        Some((pack, rel)) => {
            if !pack.is_dir(&rel) {
                return Err(not_found());
            }
            Ok(pack.children(&rel).into_iter().map(|(n, d)| (path.join(n), d)).collect())
        }
        None => Ok(std::fs::read_dir(path)?.filter_map(|e| e.ok()).map(|e| {
            let p = e.path();
            let d = p.is_dir();
            (p, d)
        }).collect()),
    }
}

// ------------------------------------------------------------------ user data

/// A folder name made from the game's name or id: `My Game!` → `my_game`.
pub fn safe_id(name: &str) -> String {
    let mut s: String = name.trim().chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '.' { c.to_ascii_lowercase() } else { '_' }).collect();
    while s.contains("__") {
        s = s.replace("__", "_");
    }
    let s = s.trim_matches('_').to_string();
    if s.is_empty() { "modengine_game".into() } else { s }
}

/// Where an exported game keeps its saves: a per-user writable folder.
///
/// - Linux/BSD: `$XDG_DATA_HOME/<id>` or `~/.local/share/<id>`
/// - Windows: `%APPDATA%\<id>`
/// - macOS: `~/Library/Application Support/<id>`
/// - Android: the app's internal storage, `/data/data/<package>/files`
///
/// `env` looks up environment variables (tests pass their own).
pub fn user_data_dir(id: &str, android_package: Option<&str>, env: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    let id = safe_id(id);
    if cfg!(target_os = "android") {
        let pkg = android_package.unwrap_or(&id);
        return PathBuf::from(format!("/data/data/{pkg}/files"));
    }
    if cfg!(windows) {
        if let Some(a) = env("APPDATA") {
            return PathBuf::from(a).join(id);
        }
    }
    if cfg!(target_os = "macos") {
        if let Some(h) = env("HOME") {
            return PathBuf::from(h).join("Library/Application Support").join(id);
        }
    }
    if let Some(x) = env("XDG_DATA_HOME").filter(|x| !x.is_empty()) {
        return PathBuf::from(x).join(id);
    }
    if let Some(h) = env("HOME").or_else(|| env("USERPROFILE")) {
        return PathBuf::from(h).join(".local/share").join(id);
    }
    std::env::temp_dir().join(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mounted_paths_read_from_the_pack() {
        let bytes = crate::pack::write(&[("game.toml".into(), b"a".to_vec()), ("sprites/x.png".into(), b"png".to_vec())]);
        let root = std::env::temp_dir().join(format!("vfs_test_{}/game.dat", std::process::id()));
        mount(&root, Pack::parse(bytes).unwrap());
        assert_eq!(read(&root.join("game.toml")).unwrap(), b"a");
        assert_eq!(read_to_string(&root.join("sprites/../game.toml")).unwrap(), "a");
        assert!(is_dir(&root) && is_dir(&root.join("sprites")) && is_file(&root.join("sprites/x.png")));
        assert!(!exists(&root.join("rooms")) && read(&root.join("nope")).is_err());
        assert_eq!(read_dir(&root.join("sprites")).unwrap(), vec![(root.join("sprites/x.png"), false)]);
        unmount(&root);
        assert!(!is_mounted(&root) && read(&root.join("game.toml")).is_err());
    }

    #[test]
    fn save_dir_selection() {
        let env = |vars: &'static [(&'static str, &'static str)]| move |k: &str| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
        let d = user_data_dir("My Game!", None, &env(&[("HOME", "/home/ana"), ("APPDATA", "C:\\Users\\ana\\AppData\\Roaming")]));
        if cfg!(windows) {
            assert_eq!(d, PathBuf::from("C:\\Users\\ana\\AppData\\Roaming").join("my_game"));
        } else if cfg!(target_os = "macos") {
            assert_eq!(d, PathBuf::from("/home/ana/Library/Application Support/my_game"));
        } else {
            assert_eq!(d, PathBuf::from("/home/ana/.local/share/my_game"));
            let x = user_data_dir("Plat", None, &env(&[("HOME", "/home/ana"), ("XDG_DATA_HOME", "/data")]));
            assert_eq!(x, PathBuf::from("/data/plat"));
        }
        assert_eq!(safe_id("  Súper  Juego 2 "), "súper_juego_2");
        assert_eq!(safe_id("!!!"), "modengine_game");
    }
}

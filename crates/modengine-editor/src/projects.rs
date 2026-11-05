//! Projects on disk, without any UI: the recent projects list (pinned, last opened,
//! missing), recognising project folders, validating a new project's folder name, where
//! new projects go by default, the shipped examples and project thumbnails.
//!
//! Everything lives in the editor's config folder (`~/.config/modengine/`): `recent.txt`
//! (one project per line: `path<TAB>pinned<TAB>unix time`; old files with just paths
//! still load), `new_project_dir.txt` (last location used) and `project_favorites.txt`.

use std::path::{Path, PathBuf};

/// `~/.config/modengine` (where the editor keeps its settings).
pub fn config_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(|h| PathBuf::from(h).join(".config/modengine"))
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ------------------------------------------------------------------ recognising projects

/// The project folder `path` stands for: a folder with a game.toml, or a game.toml file.
pub fn project_dir(path: &Path) -> Option<PathBuf> {
    if path.is_file() && path.file_name().is_some_and(|n| n == "game.toml") {
        return path.parent().map(|p| if p.as_os_str().is_empty() { PathBuf::from(".") } else { p.to_path_buf() });
    }
    path.join("game.toml").is_file().then(|| path.to_path_buf())
}

/// `[game] name` of the project in `dir` (the folder name when the file has none), or
/// `None` when `dir` is not a project.
pub fn project_name(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("game.toml")).ok()?;
    let name = text.parse::<toml::Table>().ok().and_then(|t| t.get("game")?.get("name")?.as_str().map(String::from)).filter(|n| !n.trim().is_empty());
    Some(name.unwrap_or_else(|| folder_name(dir)))
}

pub fn folder_name(dir: &Path) -> String {
    dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| dir.display().to_string())
}

/// What a dropped / pasted set of paths opens, if it is a single project.
pub fn dropped_project(paths: &[PathBuf]) -> Option<PathBuf> {
    match paths {
        [one] => project_dir(one),
        _ => None,
    }
}

// ------------------------------------------------------------------ recent projects

#[derive(Clone, Debug, PartialEq)]
pub struct Recent {
    pub path: PathBuf,
    pub pinned: bool,
    /// Unix seconds (0 = unknown, from an old recent.txt).
    pub opened: u64,
}

/// How many unpinned projects are remembered.
pub const MAX_RECENT: usize = 20;

#[derive(Clone, Debug, Default)]
pub struct Recents {
    /// `None`: in-memory only (tests, screenshots without a config folder).
    pub file: Option<PathBuf>,
    pub list: Vec<Recent>,
}

impl Recents {
    pub fn load(file: Option<PathBuf>) -> Recents {
        let text = file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
        let mut list: Vec<Recent> = vec![];
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let mut parts = line.split('\t');
            let path = PathBuf::from(parts.next().unwrap_or_default());
            let pinned = parts.next() == Some("1");
            let opened = parts.next().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
            if !list.iter().any(|r| r.path == path) {
                list.push(Recent { path, pinned, opened });
            }
        }
        Recents { file, list }
    }

    pub fn save(&self) {
        let Some(f) = &self.file else { return };
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let text: Vec<String> = self.list.iter().map(|r| format!("{}\t{}\t{}", r.path.display(), if r.pinned { 1 } else { 0 }, r.opened)).collect();
        let _ = std::fs::write(f, text.join("\n") + "\n");
    }

    fn key(path: &Path) -> PathBuf {
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
    }

    /// A project was opened now: first in the list (keeping its pin).
    pub fn touch(&mut self, path: &Path) {
        self.touch_at(path, now_secs());
    }

    pub fn touch_at(&mut self, path: &Path, when: u64) {
        let p = Self::key(path);
        let pinned = self.list.iter().any(|r| r.path == p && r.pinned);
        self.list.retain(|r| r.path != p);
        self.list.insert(0, Recent { path: p, pinned, opened: when });
        // forget the oldest unpinned ones
        let mut unpinned = 0;
        self.list.retain(|r| {
            if r.pinned {
                return true;
            }
            unpinned += 1;
            unpinned <= MAX_RECENT
        });
        self.save();
    }

    pub fn set_pinned(&mut self, path: &Path, pinned: bool) {
        if let Some(r) = self.list.iter_mut().find(|r| r.path == path) {
            r.pinned = pinned;
        }
        self.save();
    }

    pub fn remove(&mut self, path: &Path) {
        self.list.retain(|r| r.path != path);
        self.save();
    }

    /// Replaces a missing project's path with where it is now (Localizar…).
    pub fn relocate(&mut self, old: &Path, new: &Path) {
        let new = Self::key(new);
        let Some(i) = self.list.iter().position(|r| r.path == old) else { return };
        self.list.retain(|r| r.path != new || r.path == old);
        let i = self.list.iter().position(|r| r.path == old).unwrap_or(i);
        self.list[i].path = new;
        self.save();
    }

    /// Pinned first, then most recently opened; filtered by `query` (name or path).
    pub fn sorted(&self, query: &str, name_of: impl Fn(&Path) -> String) -> Vec<Recent> {
        let q = query.trim().to_lowercase();
        let mut v: Vec<Recent> = self
            .list
            .iter()
            .filter(|r| q.is_empty() || r.path.to_string_lossy().to_lowercase().contains(&q) || name_of(&r.path).to_lowercase().contains(&q))
            .cloned()
            .collect();
        // stable: entries without a time keep their file order
        v.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.opened.cmp(&a.opened)));
        v
    }

    pub fn paths(&self) -> Vec<PathBuf> {
        self.sorted("", |_| String::new()).into_iter().map(|r| r.path).collect()
    }
}

/// "hace 5 min", "ayer", "12/03/2026"… for the recent project cards.
pub fn ago(opened: u64, now: u64) -> String {
    if opened == 0 {
        return String::new();
    }
    let d = now.saturating_sub(opened);
    match d {
        0..60 => "hace un momento".into(),
        60..3600 => format!("hace {} min", d / 60),
        3600..86400 => format!("hace {} h", d / 3600),
        86400..172800 => "ayer".into(),
        172800..2592000 => format!("hace {} días", d / 86400),
        _ => {
            let (y, m, day) = civil(opened / 86400);
            format!("{day:02}/{m:02}/{y}")
        }
    }
}

/// Days since 1970-01-01 to (year, month, day) (Howard Hinnant's algorithm).
fn civil(days: u64) -> (i64, u32, u32) {
    let z = days as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

// ------------------------------------------------------------------ new projects

/// Checks a new project's folder name (it is also the game's name).
pub fn validate_name(name: &str) -> Result<(), String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("Escribe un nombre para el proyecto".into());
    }
    if n == "." || n == ".." {
        return Err("Ese nombre no vale como carpeta".into());
    }
    if n.starts_with('.') {
        return Err("No empieces el nombre por un punto (sería una carpeta oculta)".into());
    }
    if n.ends_with('.') {
        return Err("No acabes el nombre en punto (Windows no lo admite)".into());
    }
    if let Some(c) = n.chars().find(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control()) {
        return Err(format!("El nombre no puede llevar «{}»", if c.is_control() { '·' } else { c }));
    }
    if n.chars().count() > 80 {
        return Err("El nombre es demasiado largo (máximo 80 caracteres)".into());
    }
    let upper = n.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&upper.as_str()) || ((upper.starts_with("COM") || upper.starts_with("LPT")) && upper.len() == 4 && upper.as_bytes()[3].is_ascii_digit());
    if reserved {
        return Err(format!("«{n}» es un nombre reservado en Windows"));
    }
    Ok(())
}

/// Where the project would be created, or why it can't.
pub fn new_project_target(location: &str, name: &str) -> Result<PathBuf, String> {
    validate_name(name)?;
    if location.trim().is_empty() {
        return Err("Elige dónde guardar el proyecto".into());
    }
    let loc = crate::import::expand(location);
    if !loc.is_absolute() {
        return Err("La ubicación debe ser una ruta completa (por ejemplo ~/Documentos/ModEngine)".into());
    }
    if loc.exists() && !loc.is_dir() {
        return Err(format!("{} no es una carpeta", loc.display()));
    }
    let target = loc.join(name.trim());
    if target.join("game.toml").exists() {
        return Err("Ahí ya hay un proyecto de Mod Engine: ábrelo en vez de crearlo".into());
    }
    if target.exists() {
        if !target.is_dir() {
            return Err(format!("Ya existe un fichero llamado «{}»", name.trim()));
        }
        if std::fs::read_dir(&target).map(|mut d| d.next().is_some()).unwrap_or(true) {
            return Err(format!("La carpeta «{}» ya existe y no está vacía", name.trim()));
        }
    }
    Ok(target)
}

/// The Documents folder (XDG `user-dirs.dirs` on Linux, then the usual names).
pub fn documents_dir() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?);
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    if let Ok(text) = std::fs::read_to_string(xdg.join("user-dirs.dirs")) {
        for l in text.lines() {
            if let Some(v) = l.trim().strip_prefix("XDG_DOCUMENTS_DIR=") {
                let v = v.trim().trim_matches('"').replace("$HOME", &home.to_string_lossy());
                let p = PathBuf::from(v);
                if p.is_absolute() && p != home {
                    return Some(p);
                }
            }
        }
    }
    ["Documentos", "Documents"].iter().map(|n| home.join(n)).find(|p| p.is_dir()).or(Some(home))
}

/// The location offered for a new project: the last one used, else
/// `<Documentos>/ModEngine`.
pub fn default_location(config: Option<&Path>) -> PathBuf {
    let remembered = config.and_then(|c| std::fs::read_to_string(c.join("new_project_dir.txt")).ok()).map(|t| PathBuf::from(t.trim())).filter(|p| p.is_absolute());
    remembered.unwrap_or_else(|| documents_dir().unwrap_or_default().join("ModEngine"))
}

pub fn remember_location(config: Option<&Path>, loc: &Path) {
    if let Some(c) = config {
        let _ = std::fs::create_dir_all(c);
        let _ = std::fs::write(c.join("new_project_dir.txt"), loc.display().to_string());
    }
}

pub fn load_favorites(config: Option<&Path>) -> Vec<PathBuf> {
    config.and_then(|c| std::fs::read_to_string(c.join("project_favorites.txt")).ok()).map(|t| t.lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect()).unwrap_or_default()
}

pub fn save_favorites(config: Option<&Path>, favs: &[PathBuf]) {
    if let Some(c) = config {
        let _ = std::fs::create_dir_all(c);
        let text: Vec<String> = favs.iter().map(|p| p.display().to_string()).collect();
        let _ = std::fs::write(c.join("project_favorites.txt"), text.join("\n"));
    }
}

/// Screen size presets of the Nuevo proyecto dialog: (label, width, height, scale).
pub const RESOLUTIONS: &[(&str, u32, u32, u32)] = &[
    ("320 × 180 · ×4 (16:9, pixel art)", 320, 180, 4),
    ("480 × 270 · ×3 (16:9)", 480, 270, 3),
    ("640 × 360 · ×2 (16:9)", 640, 360, 2),
    ("256 × 224 · ×3 (SNES)", 256, 224, 3),
    ("160 × 144 · ×5 (Game Boy)", 160, 144, 5),
    ("1280 × 720 · ×1 (HD)", 1280, 720, 1),
];

// ------------------------------------------------------------------ examples

/// Folders that may hold the shipped examples: next to the installed app
/// (`app/examples`, see instalar.sh) and the source tree of a development build.
pub fn example_roots() -> Vec<PathBuf> {
    let mut v = vec![];
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        v.push(exe_dir.join("examples"));
        v.push(exe_dir.join("../examples"));
        v.push(exe_dir.join("../../examples")); // target/release -> repo
    }
    v.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples"));
    v
}

/// Projects inside `roots` (each root's direct subfolders with a game.toml), as
/// (game name, folder), platformer first, without duplicates.
pub fn examples_in(roots: &[PathBuf]) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = vec![];
    for root in roots {
        let Ok(rd) = std::fs::read_dir(root) else { continue };
        let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        dirs.sort();
        for d in dirs {
            let d = d.canonicalize().unwrap_or(d);
            if out.iter().any(|(_, p)| *p == d) {
                continue;
            }
            if let Some(name) = project_name(&d) {
                out.push((name, d));
            }
        }
    }
    out.sort_by_key(|(_, p)| !p.ends_with("platformer"));
    out
}

/// Copies a project folder (without saves/ and hidden folders) to `to`, which must not
/// exist or be empty.
pub fn copy_project(from: &Path, to: &Path) -> Result<(), String> {
    fn walk(from: &Path, to: &Path) -> Result<(), String> {
        std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
        for e in std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "saves" {
                continue;
            }
            let (src, dst) = (e.path(), to.join(&name));
            if src.is_dir() {
                walk(&src, &dst)?;
            } else {
                std::fs::copy(&src, &dst).map_err(|e| format!("{}: {e}", src.display()))?;
            }
        }
        Ok(())
    }
    if to.exists() && std::fs::read_dir(to).map(|mut d| d.next().is_some()).unwrap_or(true) {
        return Err(format!("{} ya existe", to.display()));
    }
    walk(from, to)
}

/// A free folder name in `loc` for a copy of `name` (`name`, `name 2`, `name 3`…).
pub fn free_folder(loc: &Path, name: &str) -> PathBuf {
    let mut p = loc.join(name);
    let mut i = 2;
    while p.exists() {
        p = loc.join(format!("{name} {i}"));
        i += 1;
    }
    p
}

// ------------------------------------------------------------------ thumbnails

/// The picture shown on a recent project's card: a screenshot the project ships
/// (`screenshot.png`, `thumbnail.png`, `icon.png`, also in `.modengine/`), else the first
/// frame of its first sprite. RGBA pixels with their size.
pub fn thumbnail(dir: &Path) -> Option<([usize; 2], Vec<u8>)> {
    for rel in ["screenshot.png", "thumbnail.png", ".modengine/screenshot.png", ".modengine/thumbnail.png", "icon.png"] {
        if let Ok(img) = image::open(dir.join(rel)) {
            let img = if img.width() > 256 || img.height() > 256 { img.thumbnail(256, 256) } else { img }.to_rgba8();
            return Some(([img.width() as usize, img.height() as usize], img.into_raw()));
        }
    }
    let game: toml::Table = std::fs::read_to_string(dir.join("game.toml")).ok()?.parse().ok()?;
    let palette: std::collections::BTreeMap<String, String> = game.get("palette").and_then(|p| p.as_table()).map(|t| t.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect()).unwrap_or_default();
    let mut files = modengine::project::asset_files(&dir.join("sprites"), &["png"]);
    files.sort_by(|a, b| a.0.cmp(&b.0));
    // prefer the player / hero if there is one
    files.sort_by_key(|(n, _, _)| !["player", "hero"].iter().any(|k| n.to_lowercase().contains(k)));
    for (name, path, is_toml) in files {
        let Ok(s) = modengine::project::load_sprite(&name, &path, is_toml, &palette) else { continue };
        let [fx, fy, fw, fh] = *s.frames.first()?;
        let (fx, fy, fw, fh) = (fx as usize, fy as usize, fw as usize, fh as usize);
        if fw == 0 || fh == 0 {
            continue;
        }
        let mut px = Vec::with_capacity(fw * fh * 4);
        for y in fy..fy + fh {
            let i = (y * s.sheet_w as usize + fx) * 4;
            px.extend_from_slice(s.rgba.get(i..i + fw * 4)?);
        }
        return Some(([fw, fh], px));
    }
    None
}

/// Opens a folder in the system's file manager.
pub fn show_in_file_manager(dir: &Path) -> Result<(), String> {
    let cmd = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(cmd).arg(dir).spawn().map(|_| ()).map_err(|e| format!("{cmd}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("modeditor_projects_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn recents_pin_remove_missing_and_legacy_format() {
        let d = temp("recents");
        let file = d.join("recent.txt");
        let (a, b, c) = (d.join("a"), d.join("b"), d.join("gone"));
        for p in [&a, &b] {
            std::fs::create_dir_all(p).unwrap();
            std::fs::write(p.join("game.toml"), "[game]\nname = \"Juego\"\n").unwrap();
        }
        // an old recent.txt: one path per line
        std::fs::write(&file, format!("{}\n{}\n", a.display(), c.display())).unwrap();
        let mut r = Recents::load(Some(file.clone()));
        assert_eq!(r.list.len(), 2);
        assert!(!r.list[1].path.exists(), "missing projects stay listed");
        r.touch_at(&b, 100);
        r.touch_at(&a, 50);
        assert_eq!(r.paths()[0], b.canonicalize().unwrap(), "most recent first");
        r.set_pinned(&a.canonicalize().unwrap(), true);
        assert_eq!(r.paths()[0], a.canonicalize().unwrap(), "pinned first");
        // persisted with pins and times
        let r2 = Recents::load(Some(file.clone()));
        assert!(r2.list.iter().any(|x| x.pinned && x.opened == 50));
        r.remove(&c);
        assert_eq!(Recents::load(Some(file.clone())).list.len(), 2);
        // search by game name or path
        assert_eq!(r.sorted("juego", |p| project_name(p).unwrap_or_default()).len(), 2);
        assert_eq!(r.sorted("/b", |_| String::new()).len(), 1);
        // reopening keeps the pin; the list is capped (pinned ones are kept)
        r.touch_at(&a, 200);
        assert!(r.list[0].pinned);
        for i in 0..30 {
            r.touch_at(&d.join(format!("p{i}")), 300 + i);
        }
        assert_eq!(r.list.iter().filter(|x| !x.pinned).count(), MAX_RECENT);
        assert!(r.list.iter().any(|x| x.pinned));
        // Localizar…
        let moved = d.join("moved");
        std::fs::create_dir_all(&moved).unwrap();
        r.relocate(&r.list[0].path.clone(), &moved);
        assert_eq!(r.list[0].path, moved.canonicalize().unwrap());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn detects_projects_and_validates_names() {
        let d = temp("detect");
        std::fs::create_dir_all(d.join("juego")).unwrap();
        std::fs::write(d.join("juego/game.toml"), "[game]\nname = \"Mi juego\"\n").unwrap();
        std::fs::create_dir_all(d.join("nada")).unwrap();
        assert_eq!(project_dir(&d.join("juego")), Some(d.join("juego")));
        assert_eq!(project_dir(&d.join("juego/game.toml")), Some(d.join("juego")));
        assert_eq!(project_dir(&d.join("nada")), None);
        assert_eq!(project_name(&d.join("juego")).as_deref(), Some("Mi juego"));
        assert_eq!(dropped_project(&[d.join("juego/game.toml")]), Some(d.join("juego")));
        assert_eq!(dropped_project(&[d.join("juego"), d.join("nada")]), None);
        // the folder browser marks project folders
        let rows = crate::agent_browser::list(&d, false).unwrap();
        assert_eq!(rows.iter().find(|r| r.name == "juego").unwrap().project.as_deref(), Some("Mi juego"));
        assert_eq!(rows.iter().find(|r| r.name == "nada").unwrap().project, None);

        for ok in ["Mi juego", "juego_2", "Pac-Man (remake)", "ñandú"] {
            assert!(validate_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", "  ", ".", "..", ".oculto", "a/b", "a\\b", "que?", "fin.", "CON", "com1", "x\ty"] {
            assert!(validate_name(bad).is_err(), "{bad:?}");
        }
        let loc = d.display().to_string();
        assert_eq!(new_project_target(&loc, "nuevo").unwrap(), d.join("nuevo"));
        assert!(new_project_target(&loc, "juego").unwrap_err().contains("ya hay un proyecto"));
        std::fs::write(d.join("nada/x.txt"), "x").unwrap();
        assert!(new_project_target(&loc, "nada").unwrap_err().contains("no está vacía"));
        std::fs::create_dir_all(d.join("vacia")).unwrap();
        assert!(new_project_target(&loc, "vacia").is_ok(), "an empty folder is fine");
        assert!(new_project_target("relativa/ruta", "x").is_err());
        assert!(new_project_target(&d.join("nada/x.txt").display().to_string(), "x").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn locations_examples_copies_and_thumbnails() {
        let d = temp("misc");
        assert!(default_location(Some(&d)).ends_with("ModEngine"));
        remember_location(Some(&d), &d.join("aqui"));
        assert_eq!(default_location(Some(&d)), d.join("aqui"));
        save_favorites(Some(&d), &[d.join("f")]);
        assert_eq!(load_favorites(Some(&d)), vec![d.join("f")]);

        // a fake examples folder, plus the real ones of the workspace
        let ex = d.join("examples");
        modengine::scaffold::create_with(&ex.join("zeta"), &Default::default()).unwrap();
        modengine::scaffold::create_with(&ex.join("platformer"), &modengine::scaffold::NewProject { template: "platformer".into(), ..Default::default() }).unwrap();
        std::fs::create_dir_all(ex.join("saves_only")).unwrap();
        let found = examples_in(&[ex.clone(), ex.clone()]);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].1.ends_with("platformer"), "platformer first");
        assert!(examples_in(&example_roots()).iter().any(|(_, p)| p.ends_with("platformer")), "the workspace examples are found");

        let copy = free_folder(&d, "zeta");
        assert_eq!(copy, d.join("zeta"));
        copy_project(&ex.join("zeta"), &copy).unwrap();
        assert!(copy.join("game.toml").exists());
        assert_eq!(free_folder(&d, "zeta"), d.join("zeta 2"));
        assert!(copy_project(&ex.join("zeta"), &copy).is_err());

        // the hero sprite becomes the thumbnail; a screenshot.png wins
        let (size, px) = thumbnail(&copy).unwrap();
        assert_eq!(size, [8, 8]);
        assert_eq!(px.len(), 8 * 8 * 4);
        image::save_buffer(copy.join("screenshot.png"), &[255u8; 4 * 4 * 4], 4, 4, image::ExtendedColorType::Rgba8).unwrap();
        assert_eq!(thumbnail(&copy).unwrap().0, [4, 4]);
        assert!(thumbnail(&d.join("nothing")).is_none());
        assert_eq!(ago(1000, 1030), "hace un momento");
        assert_eq!(ago(1000, 1000 + 7200), "hace 2 h");
        assert_eq!(ago(86400 * 365, 86400 * 400), "01/01/1971");
        assert_eq!(ago(1_772_323_200, 1_800_000_000), "01/03/2026");
        let _ = std::fs::remove_dir_all(&d);
    }
}

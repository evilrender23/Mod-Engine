//! Extensions: reusable ModScript libraries in `extensions/<name>/`, with an
//! `extension.toml` describing them. Their scripts load before the project's own (so their
//! top-level code runs first), their constants become global names, and their `init` /
//! `final` functions run when the game starts and ends.
//!
//! ```toml
//! # extensions/tween/extension.toml
//! name = "tween"
//! version = "1.0.0"
//! author = "Ana"
//! description = "Easing functions"
//! enabled = true
//! files = ["easing.ms", "tween.ms"]    # load order (default: every .ms, alphabetically)
//! init = "tween_init"                  # optional, called once at game start
//! final = "tween_final"                # optional, called when the game ends
//!
//! [constants]
//! TWEEN_VERSION = 1
//!
//! [[function]]
//! name = "ease_out_quad"
//! args = "t"
//! help = "decelerating curve, 0..1 → 0..1"
//! ```
//!
//! Native code (DLL/.so) extensions like GameMaker's are not supported: an extension is
//! ModScript only, so it runs everywhere the engine runs.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct ExtFunction {
    pub name: String,
    /// Argument list as written in a call, e.g. `"x, y, [speed]"`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub args: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub help: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct ExtensionToml {
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub enabled: bool,
    /// Script files in load order, relative to the extension folder.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub init: Option<String>,
    #[serde(rename = "final", skip_serializing_if = "Option::is_none")]
    pub final_: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub constants: BTreeMap<String, toml::Value>,
    /// Exported functions (documentation + checked to exist when the game loads).
    #[serde(rename = "function", skip_serializing_if = "Vec::is_empty")]
    pub functions: Vec<ExtFunction>,
}

impl Default for ExtensionToml {
    fn default() -> Self {
        ExtensionToml {
            name: String::new(),
            version: "1.0.0".into(),
            author: String::new(),
            description: String::new(),
            enabled: true,
            files: vec![],
            init: None,
            final_: None,
            constants: BTreeMap::new(),
            functions: vec![],
        }
    }
}

impl ExtensionToml {
    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }
}

#[derive(Clone, Debug)]
pub struct Extension {
    /// Folder name (the extension's id).
    pub name: String,
    pub dir: PathBuf,
    pub meta: ExtensionToml,
    /// Its scripts in load order, relative to the project folder (`extensions/x/a.ms`).
    pub scripts: Vec<String>,
}

pub const META_FILE: &str = "extension.toml";

/// `.ms` files of an extension folder (recursive), relative to the folder, sorted.
pub fn script_files(dir: &Path) -> Vec<String> {
    fn walk(root: &Path, d: &Path, out: &mut Vec<String>) {
        let Ok(rd) = crate::vfs::read_dir(d) else { return };
        for (p, is_dir) in rd {
            if p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
                continue;
            }
            if is_dir {
                walk(root, &p, out);
            } else if p.extension().is_some_and(|x| x == "ms") {
                out.push(p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = vec![];
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// Reads one extension folder.
pub fn load_one(project: &Path, dir: &Path) -> Result<Extension, String> {
    let name = dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let meta_path = dir.join(META_FILE);
    let meta: ExtensionToml = if crate::vfs::exists(&meta_path) {
        let text = crate::vfs::read_to_string(&meta_path).map_err(|e| format!("{}: {e}", meta_path.display()))?;
        toml::from_str(&text).map_err(|e| format!("{}: {e}", meta_path.display()))?
    } else {
        ExtensionToml { name: name.clone(), ..Default::default() }
    };
    let all = script_files(dir);
    let mut order: Vec<String> = vec![];
    for f in &meta.files {
        let f = f.replace('\\', "/");
        if !all.contains(&f) {
            return Err(format!("{}: file '{f}' listed in `files` does not exist", meta_path.display()));
        }
        order.push(f);
    }
    for f in all {
        if !order.contains(&f) {
            order.push(f);
        }
    }
    let rel_dir = dir.strip_prefix(project).unwrap_or(dir).to_string_lossy().replace('\\', "/");
    let scripts = order.into_iter().map(|f| format!("{rel_dir}/{f}")).collect();
    Ok(Extension { name, dir: dir.to_path_buf(), meta, scripts })
}

/// Every `extensions/<name>/` folder of a project, alphabetically.
pub fn load_all(project: &Path) -> Result<Vec<Extension>, String> {
    let root = project.join("extensions");
    let Ok(rd) = crate::vfs::read_dir(&root) else { return Ok(vec![]) };
    let mut dirs: Vec<PathBuf> = rd.into_iter().filter(|(p, is_dir)| *is_dir && !p.file_name().unwrap().to_string_lossy().starts_with('.')).map(|(p, _)| p).collect();
    dirs.sort();
    dirs.iter().map(|d| load_one(project, d)).collect()
}

/// Puts the scripts of enabled extensions first (in extension and `files` order) and drops
/// those of disabled extensions. `scripts` holds (relative path, source).
pub fn order_scripts(scripts: &mut Vec<(String, String)>, exts: &[Extension]) {
    let mut first = vec![];
    for e in exts {
        for rel in &e.scripts {
            if let Some(i) = scripts.iter().position(|(r, _)| r == rel) {
                let s = scripts.remove(i);
                if e.meta.enabled {
                    first.push(s);
                }
            }
        }
    }
    // any other script inside a disabled extension's folder goes too
    for e in exts.iter().filter(|e| !e.meta.enabled) {
        let prefix = format!("extensions/{}/", e.name);
        scripts.retain(|(r, _)| !r.starts_with(&prefix));
    }
    first.append(scripts);
    *scripts = first;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_and_filters() {
        let dir = std::env::temp_dir().join(format!("me_ext_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("extensions/b_lib/sub")).unwrap();
        std::fs::create_dir_all(dir.join("extensions/a_off")).unwrap();
        std::fs::write(dir.join("extensions/b_lib/z.ms"), "").unwrap();
        std::fs::write(dir.join("extensions/b_lib/sub/a.ms"), "").unwrap();
        std::fs::write(dir.join("extensions/b_lib/extension.toml"), "name = \"lib\"\nfiles = [\"z.ms\"]\n").unwrap();
        std::fs::write(dir.join("extensions/a_off/x.ms"), "").unwrap();
        std::fs::write(dir.join("extensions/a_off/extension.toml"), "enabled = false\n").unwrap();
        let exts = load_all(&dir).unwrap();
        assert_eq!(exts.len(), 2);
        assert_eq!(exts[1].scripts, ["extensions/b_lib/z.ms", "extensions/b_lib/sub/a.ms"]);
        let mut scripts: Vec<(String, String)> =
            ["extensions/a_off/x.ms", "extensions/b_lib/sub/a.ms", "extensions/b_lib/z.ms", "objects/p.ms"].iter().map(|s| (s.to_string(), String::new())).collect();
        order_scripts(&mut scripts, &exts);
        let names: Vec<&str> = scripts.iter().map(|(r, _)| r.as_str()).collect();
        assert_eq!(names, ["extensions/b_lib/z.ms", "extensions/b_lib/sub/a.ms", "objects/p.ms"]);
        std::fs::write(dir.join("extensions/b_lib/extension.toml"), "files = [\"nope.ms\"]\n").unwrap();
        assert!(load_all(&dir).unwrap_err().contains("nope.ms"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

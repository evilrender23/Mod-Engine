//! Embeds the project templates of `modengine new --template …` (scaffold.rs): the
//! shipped examples they copy and the small templates in `templates/`, so new projects
//! can be created from any installed binary. Unused data is dropped by the linker in
//! binaries that never call the scaffold (the game runner).

use std::path::{Path, PathBuf};

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
        // saves/ and hidden folders (editor layout, palette backups) are not part of a template
        if name.starts_with('.') || name == "saves" || name == "target" {
            continue;
        }
        if p.is_dir() {
            walk(root, &p, out);
        } else {
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            out.push((rel, p));
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let examples = manifest.join("../../examples");
    let sets: [(&str, PathBuf); 3] = [
        ("PLATFORMER", examples.join("platformer")),
        ("SHOWCASE", examples.join("showcase_assets_s2")),
        ("TOPDOWN", manifest.join("templates/topdown")),
    ];
    let mut code = String::new();
    for (name, dir) in &sets {
        println!("cargo:rerun-if-changed={}", dir.display());
        let mut files = vec![];
        walk(dir, dir, &mut files);
        code += &format!("pub static {name}: &[(&str, &[u8])] = &[\n");
        for (rel, path) in files {
            println!("cargo:rerun-if-changed={}", path.display());
            let abs = path.canonicalize().unwrap_or(path);
            code += &format!("    ({rel:?}, include_bytes!({:?})),\n", abs.to_string_lossy());
        }
        code += "];\n";
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("templates.rs");
    std::fs::write(out, code).unwrap();
}

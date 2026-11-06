//! `modengine sprite-bake <project> [sprite...]`: turns big text sprites (`pixels` in TOML)
//! into PNG + a short TOML, or into compact `pixels_rle` with `--rle`.

use modengine::pixelart::{bake_sprite, BakeTo};
use std::path::PathBuf;

const HELP: &str = "\
usage: modengine sprite-bake <project> [sprite...] [--rle] [--min-kb N] [--dry-run]

Converts text sprites (pixels = [...] in sprites/*.toml) to sprites/<name>.png and rewrites
the TOML with image = \"<name>.png\", keeping origin, mask, fps, rotation and the other keys.
  sprite...   only these sprites (default: every text sprite of at least --min-kb)
  --rle       write pixels_rle (run-length text) instead of a PNG
  --min-kb N  skip smaller TOML files when no sprite is named (default 8)
  --dry-run   only show what would change and the sizes";

pub fn main_cli(args: &[String]) -> i32 {
    let mut project: Option<PathBuf> = None;
    let mut names: Vec<String> = vec![];
    let (mut to, mut min_kb, mut dry) = (BakeTo::Png, 8.0f64, false);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                println!("{HELP}");
                return 0;
            }
            "--rle" => to = BakeTo::Rle,
            "--dry-run" => dry = true,
            "--min-kb" => {
                i += 1;
                match args.get(i).and_then(|v| v.parse().ok()) {
                    Some(v) => min_kb = v,
                    None => {
                        eprintln!("--min-kb necesita un número");
                        return 2;
                    }
                }
            }
            a if a.starts_with("--") => {
                eprintln!("opción desconocida {a}\n{HELP}");
                return 2;
            }
            a if project.is_none() => project = Some(PathBuf::from(a)),
            a => names.push(a.trim_end_matches(".toml").to_string()),
        }
        i += 1;
    }
    let Some(project) = project else {
        eprintln!("{HELP}");
        return 2;
    };
    let palette = match modengine::pixelart::project_palette(&project) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let dir = project.join("sprites");
    let mut files: Vec<PathBuf> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "toml")).collect(),
        Err(e) => {
            eprintln!("{}: {e}", dir.display());
            return 1;
        }
    };
    files.sort();
    for n in &names {
        if !files.iter().any(|p| p.file_stem().is_some_and(|s| s.to_string_lossy() == *n)) {
            eprintln!("no hay sprites/{n}.toml");
            return 1;
        }
    }
    let (mut before, mut after, mut count) = (0u64, 0u64, 0);
    for p in files {
        let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        if !names.is_empty() && !names.contains(&stem) {
            continue;
        }
        let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        if names.is_empty() && (size as f64) < min_kb * 1024.0 {
            continue;
        }
        if to == BakeTo::Png && p.with_extension("png").exists() {
            eprintln!("{stem}: ya existe sprites/{stem}.png, no se toca");
            continue;
        }
        match bake_sprite(&p, &palette, to, dry) {
            Ok(Some(b)) => {
                let out = match &b.png {
                    Some(png) => format!("{} + {}", p.file_name().unwrap().to_string_lossy(), png.file_name().unwrap().to_string_lossy()),
                    None => format!("{} (pixels_rle)", p.file_name().unwrap().to_string_lossy()),
                };
                println!("{stem:<24} {:>9} → {:>9}  {out}", kb(b.before), kb(b.after));
                before += b.before;
                after += b.after;
                count += 1;
            }
            Ok(None) => {
                if !names.is_empty() {
                    println!("{stem}: no es un sprite de texto (pixels), se deja igual");
                }
            }
            Err(e) => {
                eprintln!("{e}");
                return 1;
            }
        }
    }
    let verb = if dry { "se convertirían" } else { "convertidos" };
    println!("{count} sprite(s) {verb}: {} → {}", kb(before), kb(after));
    0
}

fn kb(b: u64) -> String {
    format!("{:.1} KB", b as f64 / 1024.0)
}

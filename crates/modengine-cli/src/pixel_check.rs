//! `modengine pixel-check <project> <sprite>...`: the checks of docs/pixel-art.md (sizes, rows,
//! palette, alpha, anchor, tiles) and scaled previews to look at. The work is
//! `modengine::pixelcheck`; this is the command line around it.

use modengine::pixelcheck::{self as pc, Options, Report};
use serde_json::json;
use std::path::{Path, PathBuf};

pub const USAGE: &str = "\
usage: modengine pixel-check <project> <sprite|sprites/x.toml|file.png>... [options]
       modengine pixel-check <project> --all [options]

Checks pixel art the way docs/pixel-art.md asks: every frame the same size, every row of a
text sprite (pixels = [...]) exactly as wide as the others (even when the project would not
load), characters missing from the palette (with frame, row and column, 0-based), palette
entries, colours outside the palette, binary alpha, origin and feet line of animations, empty
frames, isolated pixels and, for tiles, the seams.
  --all                 every sprite of the project
  --palette P           allowed colours: proyecto (default: game.toml [palette]; colours outside
                        are warnings), ninguna, pear36, #rrggbb,#rrggbb..., a palette file
                        (.hex .gpl .pal .json .png) or a palette saved by the editor (its name);
                        with an explicit palette, colours outside are errors
  --binary-alpha        PNG: semi-transparent pixels are errors (default: warnings)
  --size WxH            the exact frame size asked for (error when different)
  --tile                the frames are repeatable tiles: seam check and a 3x3 preview
  --preview DIR|auto    write PNG previews to DIR (auto: <temp>/modengine-pixel-check/<project>);
                        never inside the project: <name>_x<scale>.png, <name>_fondos.png,
                        <name>_bucle.png (animations), <name>_3x3.png (--tile)
  --scale N             preview scale (default: frames of at least 128 px)
  --json                machine-readable output
exit codes: 0 ok (or only warnings), 1 errors found or bad input, 2 bad arguments";

struct Args {
    project: PathBuf,
    sprites: Vec<String>,
    all: bool,
    opts: Options,
    palette: Option<String>,
    preview: Option<String>,
    scale: Option<u32>,
    json: bool,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut a = Args { project: PathBuf::new(), sprites: vec![], all: false, opts: Options::default(), palette: None, preview: None, scale: None, json: false };
    let mut pos: Vec<String> = vec![];
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        let mut value = || {
            i += 1;
            args.get(i).cloned().ok_or_else(|| format!("{flag} necesita un valor"))
        };
        match flag {
            "--all" | "--todos" => a.all = true,
            "--palette" | "--paleta" => a.palette = Some(value()?),
            "--binary-alpha" => a.opts.binary_alpha = true,
            "--tile" => a.opts.tile = true,
            "--size" => {
                let v = value()?;
                let (w, h) = v.split_once(['x', 'X', '×']).ok_or_else(|| format!("--size: usa WxH, p. ej. 16x16 (no «{v}»)"))?;
                let (w, h): (u32, u32) = (w.trim().parse().map_err(|_| format!("--size: «{v}»"))?, h.trim().parse().map_err(|_| format!("--size: «{v}»"))?);
                if w == 0 || h == 0 {
                    return Err("--size: el tamaño debe ser positivo".into());
                }
                a.opts.size = Some((w, h));
            }
            "--preview" => a.preview = Some(value()?),
            "--scale" => {
                let v = value()?;
                a.scale = Some(v.parse().ok().filter(|s: &u32| (1..=64).contains(s)).ok_or_else(|| format!("--scale: un entero de 1 a 64 (no «{v}»)"))?);
            }
            "--json" => a.json = true,
            f if f.starts_with("--") => return Err(format!("opción desconocida {f}")),
            _ => pos.push(args[i].clone()),
        }
        i += 1;
    }
    if pos.is_empty() {
        return Err("falta la carpeta del proyecto".into());
    }
    a.project = PathBuf::from(pos.remove(0));
    a.sprites = pos;
    if a.sprites.is_empty() && !a.all {
        return Err("di qué sprites revisar (nombre, sprites/x.toml o un .png) o usa --all".into());
    }
    Ok(a)
}

/// Absolute form of `p` with the existing part resolved (symlinks, `..`).
fn resolved(p: &Path) -> PathBuf {
    let abs = if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(p) };
    let mut rest = vec![];
    let mut cur = abs.as_path();
    loop {
        if let Ok(c) = cur.canonicalize() {
            let mut out = c;
            for r in rest.iter().rev() {
                out.push(r);
            }
            return out;
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                cur = parent;
            }
            _ => return abs,
        }
    }
}

/// Where `--preview auto` writes.
pub fn auto_preview_dir(project: &Path) -> PathBuf {
    let name = resolved(project).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "proyecto".into());
    std::env::temp_dir().join("modengine-pixel-check").join(name)
}

fn saved_palettes() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(|h| PathBuf::from(h).join(".config/modengine/palettes"))
}

/// Runs `modengine pixel-check`; returns the exit code and what to print (stdout, stderr).
pub fn run(args: &[String]) -> (i32, String, String) {
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        return (0, format!("{USAGE}\n"), String::new());
    }
    let mut a = match parse(args) {
        Ok(a) => a,
        Err(e) => return (2, String::new(), format!("modengine pixel-check: {e}\n\n{USAGE}\n")),
    };
    if let Some(p) = &a.palette {
        match pc::parse_palette_choice(p, saved_palettes().as_deref()) {
            Ok(c) => a.opts.palette = c,
            Err(e) => return (2, String::new(), format!("modengine pixel-check: {e}\n")),
        }
    }
    let preview = a.preview.as_ref().map(|p| if p == "auto" { auto_preview_dir(&a.project) } else { PathBuf::from(p) });
    if let Some(dir) = &preview {
        // previews are for looking, not assets: a PNG in sprites/ would become a sprite
        if resolved(dir).starts_with(resolved(&a.project)) {
            return (2, String::new(), format!("modengine pixel-check: --preview {} está dentro del proyecto; usa una carpeta temporal (o --preview auto)\n", dir.display()));
        }
    }
    let global = match modengine::pixelart::project_palette(&a.project) {
        Ok(g) => g,
        Err(e) => return (1, String::new(), format!("modengine pixel-check: {e}\n")),
    };
    let mut errors: Vec<String> = vec![];
    let mut targets: Vec<(String, PathBuf)> = if a.all { pc::all_sprites(&a.project) } else { vec![] };
    for s in &a.sprites {
        match pc::resolve(&a.project, s) {
            Ok(t) if !targets.contains(&t) => targets.push(t),
            Ok(_) => {}
            Err(e) => errors.push(e),
        }
    }
    let mut reports: Vec<Report> = vec![];
    for (name, path) in &targets {
        let mut r = pc::check_sprite(name, path, &global, &a.opts);
        if let Some(dir) = &preview {
            if let Err(e) = pc::write_previews(&mut r, dir, a.scale, a.opts.tile) {
                errors.push(e);
            }
        } else if r.art.is_some() {
            r.pending.push("no se generaron vistas previas (usa --preview DIR)".into());
        }
        reports.push(r);
    }
    let found: usize = reports.iter().map(|r| r.errors()).sum();
    let warnings: usize = reports.iter().map(|r| r.warnings()).sum();
    let code = if found > 0 || !errors.is_empty() { 1 } else { 0 };
    if a.json {
        let out = json!({
            "ok": code == 0,
            "project": a.project.display().to_string(),
            "errors": found,
            "warnings": warnings,
            "sprites": reports.iter().map(|r| r.to_json()).collect::<Vec<_>>(),
            "input_errors": errors,
        });
        return (code, serde_json::to_string_pretty(&out).unwrap_or_default() + "\n", String::new());
    }
    let mut out = String::new();
    for r in &reports {
        out += &r.to_text();
        out += "\n";
    }
    out += &format!("{} sprite(s): {found} error(es), {warnings} aviso(s)\n", reports.len());
    let err: String = errors.iter().map(|e| format!("error: {e}\n")).collect();
    (code, out, err)
}

pub fn main_cli(args: &[String]) -> i32 {
    let (code, out, err) = run(args);
    print!("{out}");
    eprint!("{err}");
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("modengine_pixcli_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sprites")).unwrap();
        std::fs::write(d.join("game.toml"), "[game]\nname = \"t\"\n[palette]\nk = \"#1a1c2c\"\ny = \"#ffcd75\"\n").unwrap();
        std::fs::write(d.join("sprites/good.toml"), "origin = \"bottom\"\npixels = ['''\n.kk.\nkyyk\n''', '''\nkyyk\n.kk.\n''']\n").unwrap();
        std::fs::write(d.join("sprites/bad.toml"), "pixels = ['''\n.kk.\nkyZk\nkyyyk\n''']\n").unwrap();
        d
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn exit_codes_and_json() {
        let d = project("codes");
        let p = d.to_str().unwrap();
        let (code, out, _) = run(&s(&[p, "good"]));
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("good (") && out.contains("pendiente"), "{out}");
        let (code, out, _) = run(&s(&[p, "bad", "--json"]));
        assert_eq!(code, 1);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let checks = v["sprites"][0]["checks"].as_array().unwrap();
        let rows = checks.iter().find(|c| c["id"] == "row_width").unwrap();
        assert_eq!(rows["status"], "error");
        assert_eq!(rows["at"][0], json!({ "frame": 0, "row": 2, "col": 4 }));
        assert_eq!(checks.iter().find(|c| c["id"] == "symbols").unwrap()["at"][0], json!({ "frame": 0, "row": 1, "col": 2 }));
        assert_eq!(run(&s(&[p, "--all"])).0, 1);
        assert_eq!(run(&s(&[p, "nada"])).0, 1, "unknown sprite");
        assert_eq!(run(&s(&[p])).0, 2, "no sprite named");
        assert_eq!(run(&s(&[p, "good", "--frobnicate"])).0, 2);
        assert_eq!(run(&s(&[p, "good", "--size", "16"])).0, 2);
        assert_eq!(run(&s(&[p, "good", "--size", "4x2"])).0, 0);
        assert_eq!(run(&s(&[p, "good", "--size", "16x16"])).0, 1);
        assert_eq!(run(&s(&[p, "good", "--palette", "#1a1c2c"])).0, 1, "explicit palette: #ffcd75 is outside");
        assert_eq!(run(&s(&[p, "good", "--palette", "no-es-color"])).0, 2);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn previews_go_outside_the_project() {
        let d = project("prev");
        let p = d.to_str().unwrap();
        let inside = d.join("sprites/previews");
        let (code, _, err) = run(&s(&[p, "good", "--preview", inside.to_str().unwrap()]));
        assert_eq!(code, 2, "{err}");
        assert!(!inside.exists());
        let out_dir = d.with_extension("previews");
        let (code, out, _) = run(&s(&[p, "good", "--preview", out_dir.to_str().unwrap(), "--json", "--tile"]));
        assert_eq!(code, 0, "{out}");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let files: Vec<String> = v["sprites"][0]["previews"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect();
        assert_eq!(files.len(), 4, "{files:?}");
        for f in &files {
            assert!(Path::new(f).is_file(), "{f}");
        }
        assert!(files[0].ends_with("good_x32.png"));
        assert!(auto_preview_dir(&d).starts_with(std::env::temp_dir().join("modengine-pixel-check")));
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&out_dir);
    }
}

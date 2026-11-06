//! `modengine import` and `modengine inspect-image`: bringing art from a folder into a project
//! without the editor (and what the editor's coding agent runs). The work itself is
//! `modengine::import`, the same code as the editor's import dialog.

use modengine::import::{self as imp, Frames, Item, Options, Origin, What};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const USAGE: &str = "\
usage:
  modengine import <project> <file|folder>... [options]
      copies PNG / WAV / OGG / TTF / OTF files into the project as sprites, tile sets, sounds
      and fonts. Folders are walked (junk such as .DS_Store, Thumbs.db, __MACOSX is skipped).
      Without options, frames come from the file name: hero_strip4.png (4 frames),
      bat_16x16.png (16x16 cells), coin_8.png (8 square frames); names with tiles/tileset/
      terrain/ts_ become tile sets (ts_<name> + its sheet spr_<name>).
    --kind sprite|tileset|sound|font   import everything as this kind
    --name N              asset name (one file only)
    --frames N            horizontal strip of N frames
    --grid WxH            frames of WxH px, in rows
    --origin O            top-left (default), center, bottom-center, top, left or x,y
    --fps F               animation speed (default 10; 0: frames are still variants)
    --tile WxH            tile size of a tile set (implies --kind tileset for PNGs)
    --offset X,Y          tile set: pixels before the first tile
    --sep X,Y             tile set: pixels between tiles
    --split-rows          with --grid: every row of the sheet becomes its own sprite
                          (<name>_row1, ...; empty cells at the end of a row are dropped)
    --row-names a,b,-,c   names of those rows, top to bottom (- skips a row); implies --split-rows
    --crop X,Y,W,H        only that part of the image (one file): a sprite out of an atlas
                          (inspect-image lists the separate drawings as \"blobs\")
    --prefix              names start with the subfolder (enemies/slime.png -> enemies_slime)
    --name-prefix P       put P in front of every name
    --overwrite           replace assets with the same name (the old files go to .trash/)
    --credit \"text\"       credit the files in CREDITS.md (author, licence, link)
    --dry-run             show what would be imported, write nothing
    --json                machine-readable output
  modengine inspect-image <file|folder> [--json]
      size, colours and likely frame layouts of PNG images, with the import options for each";

fn pair(s: &str, sep: &[char]) -> Option<[u32; 2]> {
    let (a, b) = s.split_once(sep)?;
    Some([a.trim().parse().ok()?, b.trim().parse().ok()?])
}

struct Args {
    project: PathBuf,
    inputs: Vec<PathBuf>,
    opts: Options,
    credit: Option<String>,
    dry_run: bool,
    json: bool,
}

/// One option of the importer at `args[*i]` (shared with `modengine store get`): applies it to
/// `o`, moving `i` past its value. `Ok(false)` when it is not an import option.
pub fn import_flag(args: &[String], i: &mut usize, o: &mut Options) -> Result<bool, String> {
    let flag = args[*i].as_str();
    let mut value = || {
        *i += 1;
        args.get(*i).cloned().ok_or_else(|| format!("{flag} needs a value"))
    };
    match flag {
        "--kind" => {
            let v = value()?;
            o.kind = Some(What::parse(&v).ok_or_else(|| format!("--kind: unknown kind '{v}' (sprite, tileset, sound, font)"))?);
        }
        "--name" => o.name = Some(value()?),
        "--frames" => {
            let v = value()?;
            let n: u32 = v.parse().map_err(|_| format!("--frames: '{v}' is not a number"))?;
            o.frames = Some(if n <= 1 { Frames::Single } else { Frames::Strip(n) });
        }
        "--grid" => {
            let v = value()?;
            let [w, h] = pair(&v, &['x', 'X', '×']).ok_or_else(|| format!("--grid: use WxH, e.g. 16x16 (got '{v}')"))?;
            o.frames = Some(Frames::Grid(w, h));
        }
        "--origin" => {
            let v = value()?;
            o.origin = Some(Origin::parse(&v).ok_or_else(|| format!("--origin: '{v}' (top-left, center, bottom-center, top, left or x,y)"))?);
        }
        "--fps" => {
            let v = value()?;
            let f: f64 = v.parse().map_err(|_| format!("--fps: '{v}' is not a number"))?;
            if !(f >= 0.0) {
                return Err("--fps can't be negative (0: the frames don't animate)".into());
            }
            o.fps = Some(f);
        }
        "--tile" => {
            let v = value()?;
            o.tile = Some(pair(&v, &['x', 'X', '×']).ok_or_else(|| format!("--tile: use WxH, e.g. 16x16 (got '{v}')"))?);
        }
        "--offset" => {
            let v = value()?;
            o.offset = pair(&v, &[',']).ok_or_else(|| format!("--offset: use X,Y (got '{v}')"))?;
        }
        "--sep" | "--separation" => {
            let v = value()?;
            o.sep = pair(&v, &[',']).ok_or_else(|| format!("--sep: use X,Y (got '{v}')"))?;
        }
        "--prefix" => o.prefix_dirs = true,
        "--crop" => {
            let v = value()?;
            let n: Vec<u32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            let [x, y, w, h] = n[..] else { return Err(format!("--crop: use X,Y,W,H (got '{v}')")) };
            o.crop = Some([x, y, w, h]);
        }
        "--split-rows" => o.split_rows = true,
        "--row-names" => {
            o.split_rows = true;
            o.row_names = value()?.split(',').map(|s| s.trim().to_string()).collect();
        }
        "--name-prefix" => o.name_prefix = value()?,
        "--overwrite" => o.overwrite = true,
        _ => return Ok(false),
    }
    Ok(true)
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut o = Options::default();
    let (mut credit, mut dry_run, mut json, mut pos) = (None, false, false, vec![]);
    let mut i = 0;
    while i < args.len() {
        if import_flag(args, &mut i, &mut o)? {
            i += 1;
            continue;
        }
        match args[i].as_str() {
            "--credit" => {
                i += 1;
                credit = Some(args.get(i).cloned().ok_or("--credit needs a value")?);
            }
            "--dry-run" | "-n" => dry_run = true,
            "--json" => json = true,
            f if f.starts_with("--") => return Err(format!("unknown option {f}")),
            _ => pos.push(PathBuf::from(&args[i])),
        }
        i += 1;
    }
    if o.tile.is_some() && o.kind.is_none() {
        o.kind = Some(What::Tileset);
    }
    if pos.len() < 2 {
        return Err("give the project folder and at least one file or folder to import".into());
    }
    let project = pos.remove(0);
    Ok(Args { project, inputs: pos, opts: o, credit, dry_run, json })
}

fn origin_json(o: Origin) -> Value {
    match o {
        Origin::At(x, y) => json!([x, y]),
        named => json!(named.key()),
    }
}

pub fn item_json(it: &Item, files: &[String], replaced: bool) -> Value {
    let mut v = json!({
        "kind": it.what.key(),
        "name": it.name,
        "source": it.src.display().to_string(),
        "files": files,
        "replaced": replaced,
    });
    if let Some((w, h)) = it.size {
        v["image_size"] = json!([w, h]);
    }
    if let Some(c) = it.crop {
        v["crop"] = json!(c);
    }
    if let Some((n, fw, fh)) = it.slice() {
        v["frames"] = json!(n);
        v["frame_size"] = json!([fw, fh]);
    }
    match it.what {
        What::Sprite => {
            v["origin"] = origin_json(it.opts.origin);
            v["fps"] = json!(it.opts.fps);
        }
        What::Tileset => {
            v["sprite"] = json!(it.sprite);
            if let Some(t) = it.tile {
                v["tile_size"] = json!(t.tile);
                v["offset"] = json!(t.offset);
                v["separation"] = json!(t.sep);
            }
        }
        _ => {}
    }
    if !it.why.is_empty() {
        v["detected"] = json!(it.why);
    }
    if !it.hint.is_empty() {
        v["hint"] = json!(it.hint);
    }
    if !it.warnings.is_empty() {
        v["warnings"] = json!(it.warnings);
    }
    v
}

pub fn describe(it: &Item) -> String {
    let mut d = String::new();
    match (it.what, it.slice()) {
        (What::Sprite, Some((n, fw, fh))) => {
            d += &if n == 1 { format!("1 frame de {fw}×{fh}") } else { format!("{n} frames de {fw}×{fh}") };
            if it.opts.origin != Origin::TopLeft {
                d += &match it.opts.origin {
                    Origin::At(x, y) => format!(", origen [{x}, {y}]"),
                    o => format!(", origen {}", o.key()),
                };
            }
            if it.opts.fps != imp::DEFAULT_FPS {
                d += &format!(", {} fps", it.opts.fps);
            }
        }
        (What::Tileset, Some((n, tw, th))) => d += &format!("{n} tiles de {tw}×{th}, sprite {}", it.sprite.as_deref().unwrap_or("?")),
        _ => {}
    }
    if !it.why.is_empty() {
        d += &format!(" [{}]", it.why);
    }
    d
}

/// Runs `modengine import`; returns the exit code and what to print (stdout, stderr).
pub fn run_import(args: &[String]) -> (i32, String, String) {
    if args.iter().any(|a| a == "-h" || a == "--help") || args.is_empty() {
        return (0, USAGE.to_string(), String::new());
    }
    let a = match parse(args) {
        Ok(a) => a,
        Err(e) => return (2, String::new(), format!("modengine import: {e}\n\n{USAGE}\n")),
    };
    let plan = imp::plan(&a.project, &a.inputs, &a.opts);
    let report = if a.dry_run || !plan.errors.is_empty() { imp::Report { errors: plan.errors.clone(), ..Default::default() } } else { imp::execute(&plan, a.credit.as_deref()) };
    let ok = report.errors.is_empty();
    let code = if ok { 0 } else { 1 };
    let licenses: Vec<String> = plan.license_files.iter().map(|p| p.display().to_string()).collect();
    if a.json {
        let assets: Vec<Value> = if a.dry_run {
            plan.items.iter().map(|it| item_json(it, &it.files(), it.replace)).collect()
        } else {
            report.done.iter().map(|(it, files, replaced)| item_json(it, files, *replaced)).collect()
        };
        let skipped: Vec<Value> = plan.skipped.iter().map(|(p, r)| json!({ "path": p.display().to_string(), "reason": r })).collect();
        let out = json!({
            "ok": ok,
            "dry_run": a.dry_run,
            "project": a.project.display().to_string(),
            "assets": assets,
            "skipped": skipped,
            "license_files": licenses,
            "credits": report.credits.as_ref().map(|_| imp_credits_file()),
            "errors": report.errors,
        });
        return (code, serde_json::to_string_pretty(&out).unwrap_or_default() + "\n", String::new());
    }
    let mut out = String::new();
    let mut err = String::new();
    if !ok {
        for e in &report.errors {
            err += &format!("error: {e}\n");
        }
        if report.done.is_empty() {
            err += "no se importó nada\n";
        }
    }
    let list: Vec<(&Item, Vec<String>, bool)> =
        if a.dry_run { plan.items.iter().map(|it| (it, it.files(), it.replace)).collect() } else { report.done.iter().map(|(it, f, r)| (it, f.clone(), *r)).collect() };
    if !list.is_empty() {
        out += &if a.dry_run { format!("Se importaría en {} (--dry-run: no se escribe nada):\n", a.project.display()) } else { format!("Importado en {}:\n", a.project.display()) };
        for (it, files, replaced) in &list {
            let mark = if *replaced { "~" } else { "+" };
            let src = it.src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            out += &format!("  {mark} {} {} ← {src}  {}\n", it.what.key(), it.name, describe(it));
            out += &format!("      {}{}\n", files.join(", "), if *replaced { "  (sustituye al anterior: está en .trash/)" } else { "" });
            for w in &it.warnings {
                out += &format!("      aviso: {w}\n");
            }
            if !it.hint.is_empty() {
                out += &format!("      ¿una hoja de frames? {}\n", it.hint);
            }
        }
    }
    if !plan.skipped.is_empty() {
        out += "Omitidos:\n";
        for (p, r) in &plan.skipped {
            out += &format!("  - {}: {r}\n", p.display());
        }
    }
    if !licenses.is_empty() {
        out += "Ficheros de licencia / créditos junto al arte (léelos y usa --credit \"…\"):\n";
        for l in &licenses {
            out += &format!("  - {l}\n");
        }
    }
    if report.credits.is_some() {
        out += &format!("{} actualizado\n", imp_credits_file());
    }
    (code, out, err)
}

fn imp_credits_file() -> &'static str {
    modengine::credits::FILE
}

fn layout_json(l: &imp::Layout) -> Value {
    json!({ "kind": l.kind, "frames": l.frames, "frame_size": [l.frame_w, l.frame_h], "confidence": l.confidence, "reason": l.reason, "import_args": l.args })
}

/// Runs `modengine inspect-image`.
pub fn run_inspect(args: &[String]) -> (i32, String, String) {
    if args.iter().any(|a| a == "-h" || a == "--help") || args.is_empty() {
        return (0, USAGE.to_string(), String::new());
    }
    let json_out = args.iter().any(|a| a == "--json");
    let paths: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    if paths.is_empty() {
        return (2, String::new(), format!("modengine inspect-image: give a PNG or a folder\n\n{USAGE}\n"));
    }
    let mut infos = vec![];
    let mut errors = vec![];
    for p in paths {
        let p = Path::new(p);
        if !p.exists() {
            errors.push(format!("no existe: {}", p.display()));
            continue;
        }
        let imgs = imp::images_in(p);
        if imgs.is_empty() {
            errors.push(format!("{}: no hay imágenes PNG", p.display()));
        }
        for i in imgs {
            match imp::inspect(&i) {
                Ok(info) => infos.push(info),
                Err(e) => errors.push(e),
            }
        }
    }
    let code = if errors.is_empty() { 0 } else { 1 };
    if json_out {
        let images: Vec<Value> = infos
            .iter()
            .map(|i| {
                json!({
                    "path": i.path.display().to_string(),
                    "size": [i.width, i.height],
                    "transparency": i.transparency,
                    "colors": i.colors,
                    "content_box": i.content,
                    "tileset_name": i.tileset_name,
                    "layouts": i.layouts.iter().map(layout_json).collect::<Vec<_>>(),
                    "blobs": i.blobs,
                })
            })
            .collect();
        let out = json!({ "ok": errors.is_empty(), "images": images, "errors": errors });
        return (code, serde_json::to_string_pretty(&out).unwrap_or_default() + "\n", String::new());
    }
    let mut out = String::new();
    for i in &infos {
        let colors = if i.colors > 4096 { "más de 4096".to_string() } else { i.colors.to_string() };
        out += &format!("{}: {}×{} px, {colors} colores{}\n", i.path.display(), i.width, i.height, if i.transparency { ", con transparencia" } else { ", opaca" });
        for l in &i.layouts {
            let args = if l.args.is_empty() { String::new() } else { format!("  → {}", l.args) };
            out += &format!("  {:>3.0}%  {} {} × {}×{}: {}{args}\n", l.confidence * 100.0, l.kind, l.frames, l.frame_w, l.frame_h, l.reason);
        }
        if i.blobs.len() > 1 {
            let shown: Vec<String> = i.blobs.iter().take(24).map(|b| format!("{},{},{},{}", b[0], b[1], b[2], b[3])).collect();
            out += &format!("  dibujos separados ({}; x,y,w,h para --crop): {}{}\n", i.blobs.len(), shown.join("  "), if i.blobs.len() > 24 { "  …" } else { "" });
        }
    }
    let err: String = errors.iter().map(|e| format!("error: {e}\n")).collect();
    (code, out, err)
}

/// `modengine import ...` from `main`.
pub fn main_import(args: &[String]) -> i32 {
    let (code, out, err) = run_import(args);
    print!("{out}");
    eprint!("{err}");
    code
}

/// `modengine inspect-image ...` from `main`.
pub fn main_inspect(args: &[String]) -> i32 {
    let (code, out, err) = run_inspect(args);
    print!("{out}");
    eprint!("{err}");
    code
}

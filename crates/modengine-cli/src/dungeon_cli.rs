//! `modengine dungeon <project> <config>`: generate a dungeon from the terminal to tune
//! its config (ASCII map, summary, JSON and a PNG preview). The work is
//! `modengine::dungeon`; this is the command line around it.

use modengine::dungeon::{self as dg, Config, DrawOpts, ParseCtx};
use serde_json::{json, Map, Value as J};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const USAGE: &str = "\
usage: modengine dungeon <project> <config.toml|config.json|table:id> [options]

Generates a dungeon with the config (the same struct dungeon_create() takes, see
docs/mazmorras.md) and shows it: ASCII map + summary, a PNG with --png, all the data with
--json. <config> is a file (relative to the current folder or to the project) or a record of
a data table (`dungeons:floor1` = record floor1 of data/dungeons.toml). <project> gives the
rooms used as templates and the folder for `file = ...` templates (`-` = none).
  --seed N[,M...]       seed(s) (number or text; default 1). Several seeds: one result each
  --png FILE            write a preview (one per seed: FILE gets _<seed> before .png unless it
                        contains {seed})
  --scale N             pixels per cell in the PNG (default 8)
  --show A,B            extra debug layers: delaunay, distance (also: no-mst, no-extra,
                        no-path, no-events, no-rooms)
  --json                machine-readable output (everything: rooms, graph, events...)
  --no-ascii            no ASCII map
  --strict              exit code 3 when there are warnings (missing rooms/events)
exit codes: 0 ok, 1 bad config or impossible dungeon, 2 bad arguments, 3 warnings with --strict
cells: 0 floor, 1 wall, 2 door, 3 corridor. ASCII: # wall . floor , corridor + door S start
X exit, events by the first letter of their type (lowercase)";

struct Args {
    project: String,
    config: String,
    seeds: Vec<String>,
    png: Option<String>,
    scale: u32,
    show: Vec<String>,
    json: bool,
    ascii: bool,
    strict: bool,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut a = Args { project: String::new(), config: String::new(), seeds: vec!["1".into()], png: None, scale: 8, show: vec![], json: false, ascii: true, strict: false };
    let mut pos = vec![];
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        let mut value = || {
            i += 1;
            args.get(i).cloned().ok_or_else(|| format!("{flag} necesita un valor"))
        };
        match flag {
            "--seed" | "--semilla" => a.seeds = value()?.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            "--png" => a.png = Some(value()?),
            "--scale" => a.scale = value()?.parse().map_err(|_| "--scale necesita un número")?,
            "--show" => a.show = value()?.split(',').map(|s| s.trim().to_string()).collect(),
            "--json" => a.json = true,
            "--no-ascii" => a.ascii = false,
            "--strict" => a.strict = true,
            "-h" | "--help" => return Err(String::new()),
            f if f.starts_with("--") => return Err(format!("opción desconocida {f}")),
            _ => pos.push(args[i].clone()),
        }
        i += 1;
    }
    if pos.len() != 2 {
        return Err("hacen falta <proyecto> y <config>".into());
    }
    if a.seeds.is_empty() {
        return Err("--seed: ninguna semilla".into());
    }
    a.scale = a.scale.clamp(1, 64);
    a.config = pos.pop().expect("2");
    a.project = pos.pop().expect("2");
    Ok(a)
}

/// The config as JSON: a file, or `table:id` of the project's data tables.
fn load_config(project: Option<&Path>, spec: &str) -> Result<J, String> {
    let direct = PathBuf::from(spec);
    let candidates: Vec<PathBuf> = std::iter::once(direct.clone()).chain(project.map(|p| p.join(spec))).collect();
    if let Some(path) = candidates.iter().find(|p| p.is_file()) {
        let text = std::fs::read_to_string(path).map_err(|e| format!("no se pudo leer {}: {e}", path.display()))?;
        return modengine::natives::dungeon::parse_config_text(&text, spec.ends_with(".json")).map_err(|m| format!("{}: {m}", path.display()));
    }
    if let (Some(dir), Some((table, id))) = (project, spec.split_once(':')) {
        let (tables, diags) = modengine::datatable::load_raw(dir);
        let Some(t) = tables.iter().find(|t| t.name == table) else {
            let extra = diags.first().map(|d| format!(" ({d})")).unwrap_or_default();
            return Err(format!("no hay ninguna tabla de datos '{table}' en {}{extra}", dir.join("data").display()));
        };
        let Some(r) = t.records.iter().find(|r| r.id == id) else { return Err(format!("la tabla '{table}' no tiene ningún registro '{id}'")) };
        let mut m = Map::new();
        for (k, v, _) in &r.fields {
            m.insert(k.clone(), serde_json::to_value(v).map_err(|e| e.to_string())?);
        }
        return Ok(J::Object(m));
    }
    Err(format!("no existe el fichero '{spec}' (ni en el proyecto); para una tabla de datos usa tabla:id"))
}

fn seed_of(s: &str) -> u64 {
    match s.parse::<i64>() {
        Ok(n) => n as u64,
        Err(_) => dg::seed_from_text(s),
    }
}

fn png_path(pattern: &str, seed: &str, many: bool) -> PathBuf {
    if pattern.contains("{seed}") {
        return PathBuf::from(pattern.replace("{seed}", seed));
    }
    if !many {
        return PathBuf::from(pattern);
    }
    let p = Path::new(pattern);
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "mazmorra".into());
    p.with_file_name(format!("{stem}_{seed}.png"))
}

pub fn main_cli(raw: &[String]) -> i32 {
    let a = match parse(raw) {
        Ok(a) => a,
        Err(m) => {
            if m.is_empty() {
                println!("{USAGE}");
                return 0;
            }
            eprintln!("{m}\n\n{USAGE}");
            return 2;
        }
    };
    // the project gives room templates and the folder of `file = ...` templates
    let project = (a.project != "-").then(|| PathBuf::from(&a.project));
    let engine = match &project {
        Some(p) if p.join("game.toml").is_file() => match modengine::Engine::load(p) {
            Ok(e) => Some(e),
            Err(m) => {
                eprintln!("aviso: no se pudo cargar el proyecto ({m}); las plantillas desde rooms no funcionarán");
                None
            }
        },
        Some(p) if !p.is_dir() => {
            eprintln!("{} no es una carpeta (usa - si no hay proyecto)", p.display());
            return 2;
        }
        _ => None,
    };
    let j = match load_config(project.as_deref(), &a.config) {
        Ok(j) => j,
        Err(m) => {
            eprintln!("{m}");
            return 1;
        }
    };
    let mut warnings = vec![];
    let cfg: Result<Config, String> = match &engine {
        Some(e) => modengine::natives::dungeon::parse_config(e, &j, &mut warnings),
        None => Config::parse(&j, &ParseCtx { base: project.clone(), room_loader: None }, &mut warnings),
    };
    let cfg = match cfg {
        Ok(c) => c,
        Err(m) => {
            eprintln!("configuración no válida: {m}");
            return 1;
        }
    };
    let mut o = DrawOpts::default();
    for s in &a.show {
        match s.as_str() {
            "delaunay" => o.delaunay = true,
            "distance" => o.distance = true,
            "no-mst" => o.mst = false,
            "no-extra" => o.extra = false,
            "no-path" => o.main_path = false,
            "no-events" => o.events = false,
            "no-rooms" => o.rooms = false,
            "" => {}
            other => {
                eprintln!("--show: capa desconocida '{other}' (delaunay, distance, no-mst, no-extra, no-path, no-events, no-rooms)");
                return 2;
            }
        }
    }
    let many = a.seeds.len() > 1;
    let mut out_json = vec![];
    let mut any_warnings = !warnings.is_empty();
    if !a.json {
        for w in &warnings {
            eprintln!("aviso: {w}");
        }
    }
    for s in &a.seeds {
        let t0 = Instant::now();
        let d = match dg::generate(&cfg, seed_of(s)) {
            Ok(d) => d,
            Err(m) => {
                eprintln!("semilla {s}: {m}");
                return 1;
            }
        };
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        any_warnings |= !d.warnings.is_empty();
        let png = a.png.as_ref().map(|p| png_path(p, s, many));
        if let Some(p) = &png {
            if let Some(dir) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(m) = dg::write_png(&dg::render_rgba(&d, a.scale, &o), p) {
                eprintln!("{m}");
                return 1;
            }
        }
        if a.json {
            let mut j = d.to_json();
            j["seed_text"] = json!(s);
            j["ms"] = json!((ms * 100.0).round() / 100.0);
            j["png"] = json!(png.as_ref().map(|p| p.display().to_string()));
            j["config_warnings"] = json!(warnings);
            if a.ascii {
                j["ascii"] = json!(d.ascii().lines().collect::<Vec<_>>());
            }
            out_json.push(j);
            continue;
        }
        if a.ascii {
            println!("{}", d.ascii());
        }
        let mut kinds: Vec<(String, usize)> = vec![];
        for e in &d.events {
            match kinds.iter_mut().find(|k| k.0 == e.kind) {
                Some(k) => k.1 += 1,
                None => kinds.push((e.kind.clone(), 1)),
            }
        }
        println!(
            "semilla {s}: {}×{}, {} salas, {} aristas Delaunay, {} en el árbol, {} bucles, camino principal {} casillas, {} callejones, {} intentos, {:.2} ms, hash {:016x}",
            d.w,
            d.h,
            d.rooms.len(),
            d.delaunay.len(),
            d.mst.len(),
            d.loops(),
            d.main_path.len(),
            d.dead_ends.len(),
            d.attempts,
            ms,
            d.hash()
        );
        println!("inicio {:?} (sala {}), salida {:?} (sala {})", d.start, d.start_room, d.exit, d.exit_room);
        if !kinds.is_empty() {
            println!("eventos: {}", kinds.iter().map(|(k, n)| format!("{k}×{n}")).collect::<Vec<_>>().join(", "));
        }
        for w in &d.warnings {
            eprintln!("aviso: {w}");
        }
        if let Some(p) = &png {
            println!("PNG: {}", p.display());
        }
    }
    if a.json {
        let v = if many { J::Array(out_json) } else { out_json.pop().unwrap_or(J::Null) };
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    if a.strict && any_warnings {
        3
    } else {
        0
    }
}

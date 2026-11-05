//! `modengine` command line: run, check and create Mod Engine projects.

use modengine::Engine;
use modengine_cli::script_input::ScriptedInput;
use modengine_cli::{backend, export, runner, shader_tool};
use std::path::{Path, PathBuf};
use std::process::exit;

const USAGE: &str = "\
Mod Engine — 2D games made of objects, events and rooms

usage:
  modengine run <project|game.dat> [--room R] [--exec CODE] [--input SPEC] [--screenshot <file.png>] [--frames N] [--touch]
  modengine check <project> [--room R] [--exec CODE] [--input SPEC] [--steps N] [--watch Object]
  modengine new [--template T] [--name N] <folder>
                                           new project; templates: empty, basic (default),
                                           platformer, topdown, showcase (--template list)
  modengine shader <project> [name] [--check] [--sprite S] [--uniform n=a,b] [--screenshot F.png]
                                           compile shaders on the GPU / live shader preview
  modengine export <project> --target linux|windows|android [--out DIR] [--release] [--arch A,B] [--zip]
  modengine export --setup <target>        install/check what exporting to <target> needs
  modengine export --doctor                show which export targets are ready
  modengine functions
  modengine <project>                      same as `run`

--room R       start in room R instead of the configured start room
--exec CODE    run ModScript (global scope) right after the first room starts, e.g. \"with Player x = 800\"
--input SPEC   scripted keys, e.g. \"10-200:right,30:space\" (hold right on steps 10..200, tap space at 30)
--touch        show the on-screen touch controls of [android] (the mouse acts as a finger)

in game:  F3 debug overlay   F4 fullscreen   F12 screenshot";

struct Args {
    cmd: String,
    path: Option<PathBuf>,
    screenshot: Option<String>,
    frames: Option<u64>,
    steps: u64,
    input: Option<String>,
    watch: Vec<String>,
    room: Option<String>,
    exec: Option<String>,
    touch: bool,
    template: Option<String>,
    name: Option<String>,
}

fn parse_args() -> Args {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut a = Args { cmd: String::new(), path: None, screenshot: None, frames: None, steps: 600, input: None, watch: vec![], room: None, exec: None, touch: false, template: None, name: None };
    let mut i = 0;
    let mut positional = Vec::new();
    while i < raw.len() {
        let flag = raw[i].as_str();
        let mut value = || {
            i += 1;
            raw.get(i).cloned().unwrap_or_else(|| {
                eprintln!("{flag} needs a value");
                exit(2)
            })
        };
        match flag {
            "--screenshot" => a.screenshot = Some(value()),
            "--frames" => a.frames = value().parse().ok(),
            "--steps" => a.steps = value().parse().unwrap_or(600),
            "--input" => a.input = Some(value()),
            "--watch" => a.watch.push(value()),
            "--room" => a.room = Some(value()),
            "--exec" => a.exec = Some(value()),
            "--touch" => a.touch = true,
            "--template" | "-t" => a.template = Some(value()),
            "--name" => a.name = Some(value()),
            "-h" | "--help" => {
                println!("{USAGE}");
                exit(0)
            }
            _ => positional.push(raw[i].clone()),
        }
        i += 1;
    }
    let mut pos = positional.into_iter();
    a.cmd = pos.next().unwrap_or_default();
    a.path = pos.next().map(PathBuf::from);
    if !matches!(a.cmd.as_str(), "run" | "check" | "new" | "functions") && !a.cmd.is_empty() {
        a.path = Some(PathBuf::from(&a.cmd));
        a.cmd = "run".into();
    }
    a
}

fn load(path: &Path, args: &Args) -> Engine {
    // a folder is a project; a file is an exported game (game.dat or a runner with a pack)
    let loaded = if path.is_file() { Engine::load_pack(path) } else { Engine::load(path) };
    let mut e = match loaded {
        Ok(e) => e,
        Err(msg) => {
            eprintln!("could not load the project:\n{msg}");
            exit(1)
        }
    };
    if let Some(r) = &args.room {
        match e.rooms.iter().position(|x| &x.name == r) {
            Some(i) => e.start_room = i,
            None => {
                eprintln!("--room: there is no room named '{r}'");
                exit(2)
            }
        }
    }
    if let Some(code) = &args.exec {
        if let Err(msg) = e.set_startup_code(code) {
            eprintln!("--exec: {msg}");
            exit(2)
        }
    }
    e
}

fn scripted(spec: &Option<String>) -> Option<ScriptedInput> {
    spec.as_ref().map(|s| {
        ScriptedInput::parse(s).unwrap_or_else(|e| {
            eprintln!("--input: {e}");
            exit(2)
        })
    })
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.first().map(String::as_str) == Some("shader") {
        return shader_tool::main_cli(&raw[1..]);
    }
    if raw.first().map(String::as_str) == Some("export") {
        exit(export::main_cli(&raw[1..]));
    }
    let args = parse_args();
    match args.cmd.as_str() {
        "run" => {
            let path = args.path.clone().unwrap_or_else(|| PathBuf::from("."));
            let engine = load(&path, &args);
            let shot = args.screenshot.clone().map(|p| (args.frames.unwrap_or(120), p));
            let opts = backend::RunOpts { screenshot: shot, input: scripted(&args.input), touch: args.touch.then_some(true) };
            runner::play(engine, opts);
        }
        "check" => {
            let path = args.path.clone().unwrap_or_else(|| PathBuf::from("."));
            check(&path, &args);
        }
        "new" => {
            if args.template.as_deref() == Some("list") {
                for t in modengine::scaffold::TEMPLATES {
                    println!("{:<11} {}", t.id, t.description);
                }
                return;
            }
            let Some(path) = args.path else {
                eprintln!("usage: modengine new [--template T] [--name N] <folder>");
                exit(2)
            };
            let opts = modengine::scaffold::NewProject { template: args.template.clone().unwrap_or_default(), name: args.name.clone(), ..Default::default() };
            if let Err(e) = modengine::scaffold::create_with(&path, &opts) {
                eprintln!("{e}");
                exit(1)
            }
            println!("created {}. Run it with:\n  modengine run {}", path.display(), path.display());
        }
        "functions" => {
            for n in modengine::natives::names() {
                println!("{n}");
            }
        }
        _ => println!("{USAGE}"),
    }
}

/// Loads the project and runs it without a window, reporting script errors.
fn check(path: &Path, args: &Args) {
    let mut e = load(path, args);
    println!(
        "loaded '{}': {} objects, {} rooms, {} sprites, {} sounds",
        e.config.game.name,
        e.objects.len(),
        e.rooms.len(),
        e.sprites.len(),
        e.sounds.len()
    );
    let input = scripted(&args.input);
    e.start();
    for _ in 0..args.steps {
        e.input.down.iter_mut().for_each(|d| *d = false);
        if let Some(si) = &input {
            si.apply(e.steps + 1, &mut e.input);
        }
        e.tick();
        e.render();
        e.audio.clear();
        if !args.watch.is_empty() && e.steps % 30 == 0 {
            for w in &args.watch {
                let found: Vec<String> = e
                    .insts
                    .iter()
                    .filter(|i| i.alive && e.objects[i.object as usize].name == *w)
                    .map(|i| format!("({:.1}, {:.1})", i.x, i.y))
                    .collect();
                println!("step {:>5} room {:<10} {w}: {}", e.steps, e.rooms[e.room].name, found.join(" "));
            }
        }
        if e.quit {
            println!("game called quit() at step {}", e.steps);
            break;
        }
    }
    let mut counts: Vec<(String, usize)> = Vec::new();
    for i in e.insts.iter().filter(|i| i.alive) {
        let n = &e.objects[i.object as usize].name;
        match counts.iter_mut().find(|(k, _)| k == n) {
            Some(c) => c.1 += 1,
            None => counts.push((n.clone(), 1)),
        }
    }
    println!(
        "after {} steps: room '{}', instances: {}",
        e.steps,
        e.rooms[e.room].name,
        counts.iter().map(|(n, c)| format!("{n}×{c}")).collect::<Vec<_>>().join(", ")
    );
    if e.errors.is_empty() {
        println!("no runtime errors");
    } else {
        eprintln!("{} runtime error(s)", e.errors.len());
        exit(1);
    }
}

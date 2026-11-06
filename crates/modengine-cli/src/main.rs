//! `modengine` command line: run, check and create Mod Engine projects.

use modengine::Engine;
use modengine_cli::script_input::ScriptedInput;
use modengine_cli::{backend, export, import_cli, runner, shader_tool};
use std::path::{Path, PathBuf};
use std::process::exit;

const USAGE: &str = "\
Mod Engine — 2D games made of objects, events and rooms

usage:
  modengine run <project|game.dat> [--room R] [--exec CODE] [--input SPEC] [--screenshot <file.png>] [--frames N] [--touch]
  modengine check <project> [--room R] [--exec CODE] [--input SPEC] [--steps N] [--watch Object]
  modengine server <project|game.dat> [--room R] [--exec CODE] [--input SPEC] [--steps N] [--watch Object]
                                           run the game without a window, in real time (dedicated
                                           multiplayer server; same as `run --headless`)
  modengine new [--template T] [--name N] <folder>
                                           new project; templates: empty, basic (default),
                                           platformer, topdown, showcase (--template list)
  modengine shader <project> [name] [--check] [--sprite S] [--uniform n=a,b] [--screenshot F.png]
                                           compile shaders on the GPU / live shader preview
  modengine export <project> --target linux|windows|android [--out DIR] [--release] [--arch A,B] [--zip]
  modengine export --setup <target>        install/check what exporting to <target> needs
  modengine export --doctor                show which export targets are ready
  modengine import <project> <file|folder>... [--kind K] [--name N] [--frames N | --grid WxH]
                   [--origin O] [--fps F] [--tile WxH] [--offset X,Y] [--sep X,Y] [--prefix]
                   [--overwrite] [--credit TEXT] [--dry-run] [--json]
                                           copy art into the project as sprites, tile sets,
                                           sounds and fonts (modengine import --help)
  modengine sprite-bake <project> [sprite...] [--rle] [--min-kb N] [--dry-run]
                                           turn big text sprites (pixels = [...]) into PNG
                                           (or compact pixels_rle) keeping origin/mask/fps
  modengine inspect-image <file|folder> [--json]
                                           size, colours and likely frame layouts of PNGs
  modengine store search|show|download|get ...
                                           free assets from OpenGameArt.org: search, look,
                                           import with credits (modengine store --help)
  modengine functions
  modengine <project>                      same as `run`

--room R       start in room R instead of the configured start room
--exec CODE    run ModScript (global scope) right after the first room starts, e.g. \"with Player x = 800\"
--input SPEC   scripted keys, e.g. \"10-200:right,30:space\" (hold right on steps 10..200, tap space at 30);
               \"40:text=Ana\" types Ana at step 40 (keyboard_string)
--headless     run: no window, no drawing (multiplayer servers, tests); --steps N stops after N steps
--instance N   run: window N of several copies (title [N], placed side by side) for multiplayer tests
--touch        show the on-screen touch controls of [android] (the mouse acts as a finger)

testing (run, check, server; see docs/pruebas.md):
--input SPEC          also the mouse: \"10:mouse=320,200, 20-80:mb_left, 90:wheel_up, 30:mouse_room=500,60\"
--screenshot-at A,B   PNG of the game at logic steps A, B... (shot_0200.png; --screenshot PATTERN
                      sets the name, e.g. out/s_%04d.png); hidden window, vsync off, one step per frame
--crop X,Y,W,H        crop those screenshots (game pixels)
--hidden / --show     no window on the desktop / force a visible one
--steps N             run: stop after N logic steps (lockstep, also with a window)
--seed N              reproducible rand/irand/choose/shuffle/chance/particles (randomize() too)
--exec-at STEP:CODE   run CODE after logic step STEP (repeatable; 0 = after the first room starts)
--assert [STEP:]EXPR  fail with exit code 3 if EXPR is false (at the end, after STEP, or *: every step)
--save-dir DIR        save_data/file functions use DIR;  --no-save: an empty temporary folder
--test-script F.ms    load extra objects/functions (repeatable); its objects appear in every room
--profile             time per event and per object (table at the end)
--render              check: also draw on the GPU (hidden window), needed for --screenshot-at
exit codes: 0 ok, 1 runtime/load errors, 2 bad arguments, 3 an --assert failed

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
    headless: bool,
    steps_set: bool,
    instance: Option<u32>,
    test: modengine_cli::test_run::TestArgs,
}

fn parse_args() -> Args {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut a = Args { cmd: String::new(), path: None, screenshot: None, frames: None, steps: 600, input: None, watch: vec![], room: None, exec: None, touch: false, template: None, name: None, headless: false, steps_set: false, instance: None, test: Default::default() };
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
            "--steps" => {
                a.steps = value().parse().unwrap_or(600);
                a.steps_set = true;
            }
            "--headless" => a.headless = true,
            "--instance" => a.instance = value().parse().ok(),
            "--input" => a.input = Some(value()),
            "--watch" => a.watch.push(value()),
            "--room" => a.room = Some(value()),
            "--exec" => a.exec = Some(value()),
            "--touch" => a.touch = true,
            "--template" | "-t" => a.template = Some(value()),
            "--name" => a.name = Some(value()),
            f if modengine_cli::test_run::flag_kind(f).is_some() => {
                let v = if modengine_cli::test_run::flag_kind(f) == Some(true) { Some(value()) } else { None };
                if let Err(m) = a.test.set(flag, v) {
                    eprintln!("{m}");
                    exit(2)
                }
            }
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
    if !matches!(a.cmd.as_str(), "run" | "check" | "new" | "functions" | "server") && !a.cmd.is_empty() {
        a.path = Some(PathBuf::from(&a.cmd));
        a.cmd = "run".into();
    }
    a
}

fn load(path: &Path, args: &Args) -> Engine {
    // a folder is a project; a file is an exported game (game.dat or a runner with a pack)
    let loaded = modengine::testing::load_with_tests(path, &args.test.test_scripts);
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
    if let Err(msg) = args.test.apply(&mut e) {
        eprintln!("{msg}");
        exit(2)
    }
    e
}

/// Leaves (removing the `--no-save` folder first).
fn finish(code: i32) -> ! {
    modengine_cli::test_run::cleanup();
    exit(code)
}

/// A windowed test run: `--screenshot-at`, `--hidden`, or `run --steps N` (lockstep).
fn window_test(a: &Args, steps: Option<u64>, force: bool) -> Option<modengine_cli::test_run::WindowTest> {
    let t = &a.test;
    if !(force || !t.shots.is_empty() || steps.is_some() || t.hidden) {
        return None;
    }
    let hidden = (t.hidden || !t.shots.is_empty() || force) && !t.show;
    // with screenshots the run ends at the last one, but not before the steps named by
    // --exec-at / --assert
    let steps = if t.shots.is_empty() { steps } else { steps.max(t.last_named_step()) };
    Some(modengine_cli::test_run::WindowTest::new(&t.shots, a.screenshot.clone(), t.crop, steps, hidden))
}

fn play(engine: Engine, args: &Args, test: Option<modengine_cli::test_run::WindowTest>) -> ! {
    let shot = if test.as_ref().is_some_and(|t| !t.shots.is_empty()) { None } else { args.screenshot.clone().map(|p| (args.frames.unwrap_or(120), p)) };
    let opts = backend::RunOpts { screenshot: shot, input: scripted(&args.input), touch: args.touch.then_some(true), instance: args.instance, test };
    runner::play(engine, opts);
    finish(modengine_cli::test_run::exit_code())
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
    if raw.first().map(String::as_str) == Some("import") {
        exit(import_cli::main_import(&raw[1..]));
    }
    if raw.first().map(String::as_str) == Some("sprite-bake") {
        exit(modengine_cli::sprite_bake::main_cli(&raw[1..]));
    }
    if raw.first().map(String::as_str) == Some("inspect-image") {
        exit(import_cli::main_inspect(&raw[1..]));
    }
    if raw.first().map(String::as_str) == Some("store") {
        exit(modengine_cli::store_cli::main_cli(&raw[1..]));
    }
    let args = parse_args();
    match args.cmd.as_str() {
        "run" => {
            let path = args.path.clone().unwrap_or_else(|| PathBuf::from("."));
            let engine = load(&path, &args);
            if args.headless {
                finish(headless(engine, &args));
            }
            let test = window_test(&args, args.steps_set.then_some(args.steps), false);
            play(engine, &args, test);
        }
        "server" => {
            let path = args.path.clone().unwrap_or_else(|| PathBuf::from("."));
            let engine = load(&path, &args);
            finish(headless(engine, &args));
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

/// `server` / `run --headless`: the game logic in real time, no window.
fn headless(e: Engine, args: &Args) -> i32 {
    let opts = modengine_cli::headless::HeadlessOpts { input: scripted(&args.input), steps: args.steps_set.then_some(args.steps), watch: args.watch.clone(), realtime: true };
    modengine_cli::headless::run(e, opts)
}

/// Loads the project and runs it without a window, reporting script errors.
fn check(path: &Path, args: &Args) {
    let mut e = load(path, args);
    if args.test.render || !args.test.shots.is_empty() {
        // GPU rendering in a hidden window (screenshots at exact steps)
        let steps = (args.steps_set || args.test.shots.is_empty()).then_some(args.steps);
        let test = window_test(args, steps, true);
        play(e, args, test);
    }
    println!(
        "loaded '{}': {} objects, {} rooms, {} sprites, {} sounds",
        e.config.game.name,
        e.objects.len(),
        e.rooms.len(),
        e.sprites.len(),
        e.sounds.len()
    );
    let lint_errors = print_lint(&e);
    let input = scripted(&args.input);
    e.test_starting();
    e.start();
    e.test_started();
    for _ in 0..args.steps {
        e.input.down.iter_mut().for_each(|d| *d = false);
        e.input.mouse_down = [false; 3];
        if let Some(si) = &input {
            si.apply_to(e.steps + 1, &mut e);
        }
        e.test_tick();
        e.test_render();
        e.audio.clear();
        if !args.watch.is_empty() && e.steps % 30 == 0 {
            for l in modengine_cli::headless::watch_lines(&e, &args.watch) {
                println!("{l}");
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
    let code = e.test_finish();
    if e.error_count() == 0 {
        println!("no runtime errors");
    } else {
        eprintln!("{} runtime error(s)", e.error_count());
    }
    finish(if code == 0 && lint_errors > 0 { 1 } else { code })
}

/// Static analysis (see `modengine::lint`): prints findings, returns the error count.
fn print_lint(e: &Engine) -> usize {
    let found = modengine::lint::check(e);
    let errors = found.iter().filter(|f| f.severity == modengine::lint::Severity::Error).count();
    for f in &found {
        println!("{f}");
    }
    if found.is_empty() {
        println!("static analysis: no problems found");
    } else {
        println!("static analysis: {errors} error(s), {} warning(s)", found.len() - errors);
    }
    errors
}

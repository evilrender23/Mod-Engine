//! Paths, timelines, animation curves and sequences: loaded from a project and run
//! headless for some steps, like `modengine check`.

use modengine::draw::DrawCmd;
use modengine::Engine;
use modscript::{intern, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static N: AtomicU32 = AtomicU32::new(0);

fn project(files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("modengine-motion-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("game.toml"), "[game]\nname = \"t\"\n\n[display]\nwidth = 320\nheight = 180\nstep_rate = 60\n\n[palette]\n\"w\" = \"#ffffff\"\n").unwrap();
    let mut all: Vec<(&str, &str)> = vec![("sprites/dot.toml", "origin = \"center\"\npixels = ['''\nww\nww\n''']\n"), ("rooms/r.toml", "width = 320\nheight = 180\n[[layer]]\nname = \"Instances\"\ntype = \"instances\"\n")];
    all.extend_from_slice(files);
    for (rel, text) in all {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

fn run(dir: &PathBuf, steps: usize) -> Engine {
    let mut e = Engine::load(dir).unwrap_or_else(|e| panic!("{e}"));
    e.start();
    for _ in 0..steps {
        e.tick();
        e.render();
    }
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    e
}

fn global(e: &Engine, n: &str) -> Value {
    e.globals.get(&intern(n)).cloned().unwrap_or(Value::Null)
}

fn num(e: &Engine, n: &str) -> f64 {
    global(e, n).as_num().unwrap_or_else(|| panic!("global.{n} is not a number: {}", global(e, n)))
}

fn inst<'a>(e: &'a Engine, object: &str) -> &'a modengine::instance::Instance {
    e.insts.iter().find(|i| e.objects[i.object as usize].name == object).unwrap_or_else(|| panic!("no {object}"))
}

const SQUARE: &str = "closed = true\n[[point]]\nx = 0\ny = 0\n[[point]]\nx = 100\ny = 0\n[[point]]\nx = 100\ny = 100\n[[point]]\nx = 0\ny = 100\n";
const LINE: &str = "[[point]]\nx = 0\ny = 0\n[[point]]\nx = 60\ny = 0\nspeed = 50\n[[point]]\nx = 120\ny = 0\n";

#[test]
fn instances_follow_paths() {
    let objs = r#"
object Walker {
    sprite = dot
    on create { path_start(square, 25, path_action.restart, true) }
}
object Stopper {
    on create { path_start(line, 10, path_action.stop, false) }
    on path_ended { global.ended_at = x }
}
object Pong {
    on create { path_start(line, 30, path_action.reverse, true) }
}
global.len = path_get_length(square)
global.mid_x = path_get_x(square, 0.375)
global.mid_y = path_get_y(square, 0.375)
global.kind = path_get_kind(square)
var p = path_add()
path_add_point(p, 0, 0)
path_add_point(p, 30, 40)
global.dyn_len = path_get_length(p)
path_shift(p, 10, 10)
global.dyn_x0 = path_get_point_x(p, 0)
global.n = path_get_number(square)
"#;
    let room = "width = 320\nheight = 180\n[[layer]]\ntype = \"instances\"\ninstances = [ { object = \"Walker\", x = 0, y = 0 }, { object = \"Stopper\", x = 200, y = 50 }, { object = \"Pong\", x = 0, y = 0 } ]\n";
    let dir = project(&[("paths/square.toml", SQUARE), ("paths/line.toml", LINE), ("objects.ms", objs), ("rooms/r.toml", room)]);
    let e = run(&dir, 6);
    assert_eq!(num(&e, "len"), 400.0);
    assert_eq!((num(&e, "mid_x"), num(&e, "mid_y")), (100.0, 50.0));
    assert_eq!(num(&e, "kind"), 0.0);
    assert_eq!(num(&e, "dyn_len"), 50.0);
    assert_eq!(num(&e, "dyn_x0"), 10.0);
    assert_eq!(num(&e, "n"), 4.0);
    // 6 steps of 25 px along the square: (100, 50)
    let w = inst(&e, "Walker");
    assert_eq!((w.x, w.y), (100.0, 50.0));
    assert!((w.path.position - 0.375).abs() < 1e-9);
    assert_eq!(w.dir, 270.0, "moving down");
    // the point speed (50 % at x=60) slows the stopper down in the middle of its line
    let s = inst(&e, "Stopper");
    assert!(s.x > 200.0 && s.x < 260.0, "{}", s.x);
    // the pong reached the end of its line on step 6 and turned back
    let p = inst(&e, "Pong");
    assert!(p.path.speed < 0.0, "reverse flipped the speed");
    // keep going until the stopper ends its path and the walker loops
    let mut e = e;
    for _ in 0..40 {
        e.tick();
    }
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    assert_eq!(num(&e, "ended_at"), 320.0, "path_ended fires at the end of a relative path");
    let s = inst(&e, "Stopper");
    assert!(matches!(s.path.index, None));
    let w = inst(&e, "Walker");
    assert!(w.path.index.is_some(), "restart keeps going");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn timelines_run_moments_in_the_instance() {
    let objs = r#"
object Actor {
    count = 0
    on create {
        timeline_index = intro
        timeline_running = true
        timeline_loop = true
    }
}
object Builder {
    hits = 0
    on create {
        var t = timeline_add()
        timeline_moment_add_script(t, 2, function() { hits += 1 })
        timeline_start(t, 1, false)
        global.size = timeline_size(t)
    }
}
"#;
    let tl = "[[moment]]\nstep = 0\ncode = \"count += 1\"\n\n[[moment]]\nstep = 3\ncode = '''\ncount += 10\nglobal.pos_in_moment = timeline_position\n'''\n";
    let room = "width = 64\nheight = 64\n[[layer]]\ntype = \"instances\"\ninstances = [ { object = \"Actor\", x = 0, y = 0 }, { object = \"Builder\", x = 0, y = 0 } ]\n";
    let dir = project(&[("timelines/intro.toml", tl), ("objects.ms", objs), ("rooms/r.toml", room)]);
    // period of a looping timeline = last moment + 1 = 4 steps; 9 steps = moments 0,3,0,3,0
    let e = run(&dir, 9);
    let a = inst(&e, "Actor");
    assert_eq!(a.vars.get(&intern("count")).and_then(|v| v.as_num()), Some(23.0));
    assert_eq!(a.timeline.position, 1.0);
    assert_eq!(num(&e, "pos_in_moment"), 0.0, "position wraps before the moment runs");
    let b = inst(&e, "Builder");
    assert_eq!(b.vars.get(&intern("hits")).and_then(|v| v.as_num()), Some(1.0));
    assert_eq!(num(&e, "size"), 1.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn animation_curves_evaluate() {
    let curve = "[[channel]]\nname = \"x\"\ntype = \"linear\"\npoints = [[0, 0], [1, 10]]\n\n[[channel]]\nname = \"y\"\ntype = \"smooth\"\npoints = [[0, 0], [0.5, 1], [1, 0]]\n";
    let init = r#"
global.a = animcurve_evaluate(wave, "x", 0.25)
global.b = animcurve_channel_evaluate(animcurve_get_channel(wave, "y"), 0.5)
global.c = animcurve_channel_evaluate(animcurve_get_channel(wave, 0), 2)
var cur = animcurve_get(wave)
global.names = cur.channels[0].name + cur.channels[1].name
global.idx = animcurve_get_channel_index(wave, "y")
var ch = animcurve_channel_new()
ch.type = "linear"
ch.points = [{ posx: 0, value: 5 }, { posx: 1, value: 7 }]
global.d = animcurve_channel_evaluate(ch, 0.5)
global.exists = animcurve_exists(wave)
"#;
    let dir = project(&[("curves/wave.toml", curve), ("scripts/init.ms", init)]);
    let e = run(&dir, 1);
    assert_eq!(num(&e, "a"), 2.5);
    assert_eq!(num(&e, "b"), 1.0);
    assert_eq!(num(&e, "c"), 10.0, "clamped past the last point");
    assert_eq!(global(&e, "names").to_string(), "xy");
    assert_eq!(num(&e, "idx"), 1.0);
    assert_eq!(num(&e, "d"), 6.0);
    assert!(global(&e, "exists").truthy());
    let _ = std::fs::remove_dir_all(&dir);
}

const SEQ: &str = r#"
length = 30
fps = 60
playback = "oneshot"

[[track]]
name = "spark"
type = "sprite"
asset = "dot"
[[track.key]]
frame = 0
x = 0
y = 0
alpha = 1
[[track.key]]
frame = 30
x = 30
y = 0
alpha = 0

[[track]]
name = "mover"
type = "object"
asset = "Puppet"
[[track.key]]
frame = 0
x = 0
y = 0
ease = "fast"
[[track.key]]
frame = 20
x = 0
y = 40

[[moment]]
frame = 10
code = "global.moment_hit += 1"
"#;

#[test]
fn sequences_play_tracks_and_moments() {
    let objs = r#"
object Puppet { sprite = dot }
global.moment_hit = 0
"#;
    // a curve used as easing: t² (with the name of the curve in `ease`)
    let curve = "[[channel]]\nname = \"c\"\ntype = \"bezier\"\npoints = [[0, 0, 0, 0, 0.5, 0], [1, 1, -0.0001, -0.0001, 0, 0]]\n";
    let room = "width = 320\nheight = 180\ncode = '''\nglobal.el = layer_sequence_create(\"Instances\", 100, 50, intro)\nglobal.el2 = sequence_create(intro, 0, 0, 10)\nlayer_sequence_speedscale(global.el2, 2)\n'''\n[[layer]]\nname = \"Instances\"\ntype = \"instances\"\n";
    let dir = project(&[("sequences/intro.toml", SEQ), ("curves/fast.toml", curve), ("objects.ms", objs), ("rooms/r.toml", room)]);
    let mut e = run(&dir, 10);
    let el = num(&e, "el") as u32;
    let s = e.seqs.get(el).unwrap();
    assert!((s.head - 10.0).abs() < 1e-9, "60 fps sequence in a 60 steps/s room: 1 frame per step");
    // the object track created a Puppet and moves it (eased by the curve: slower than linear)
    let pup = e.insts.iter().filter(|i| e.objects[i.object as usize].name == "Puppet").map(|i| i.y).collect::<Vec<_>>();
    assert_eq!(pup.len(), 2, "one puppet per playing sequence");
    let y = e.insts.iter().find(|i| e.objects[i.object as usize].name == "Puppet" && i.x == 100.0).unwrap().y;
    assert!(y > 50.0 && y < 70.0, "eased below the linear 70: {y}");
    assert_eq!(num(&e, "moment_hit"), 1.0, "el2 at double speed crossed frame 10; el crosses it on the next step");
    // the sprite track is drawn interpolated: x = 100 + 10, alpha 1 - 10/30
    let spr: Vec<(f32, f32)> = e.cmds.iter().filter_map(|c| match c { DrawCmd::Sprite { x, color, .. } => Some((*x, color.a)), _ => None }).collect();
    assert!(spr.iter().any(|(x, a)| (*x - 110.0).abs() < 1e-3 && (*a - 2.0 / 3.0).abs() < 1e-3), "{spr:?}");
    for _ in 0..25 {
        e.tick();
    }
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    assert_eq!(num(&e, "moment_hit"), 2.0);
    let s = e.seqs.get(el).unwrap();
    assert!(s.finished && s.head == 30.0);
    // play again from the start, then destroy: its puppet goes away
    let code = modscript::parse_stmts("t", "layer_sequence_play(global.el)\nglobal.fin = layer_sequence_is_finished(global.el)\nglobal.len = layer_sequence_get_length(global.el)\nlayer_sequence_destroy(global.el2)\nglobal.ex = layer_sequence_exists(global.el2)").unwrap();
    modscript::run_stmts(&mut e, &code, &std::rc::Rc::from("t"), modscript::CallCx::global()).unwrap();
    e.tick();
    assert!(!global(&e, "fin").truthy());
    assert_eq!(num(&e, "len"), 30.0);
    assert!(!global(&e, "ex").truthy());
    assert_eq!(e.insts.iter().filter(|i| e.objects[i.object as usize].name == "Puppet").count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_assets_report_errors() {
    let dir = project(&[("sequences/s.toml", "[[track]]\nname = \"t\"\ntype = \"object\"\nasset = \"Nope\"\n")]);
    let err = Engine::load(&dir).err().expect("missing object must fail");
    assert!(err.contains("object 'Nope' does not exist"), "{err}");
    let dir2 = project(&[("timelines/t.toml", "[[moment]]\nstep = 1\ncode = \"x = (\"\n")]);
    let err = Engine::load(&dir2).err().expect("bad code must fail");
    assert!(err.contains("moment 1"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir2);
}

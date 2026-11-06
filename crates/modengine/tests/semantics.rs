//! Behaviours documented in docs/lenguaje.md ("Detalles que conviene saber"):
//! each claim there is checked here.

use modengine::Engine;
use modscript::intern;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static N: AtomicU32 = AtomicU32::new(0);

fn project(script: &str, rooms: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("modengine-sem-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("rooms")).unwrap();
    std::fs::write(dir.join("game.toml"), "[game]\nname = \"t\"\n\n[display]\nwidth = 100\nheight = 80\n").unwrap();
    std::fs::write(dir.join("main.ms"), script).unwrap();
    for (name, toml) in rooms {
        std::fs::write(dir.join(format!("rooms/{name}.toml")), toml).unwrap();
    }
    dir
}

fn load(script: &str, rooms: &[(&str, &str)]) -> Engine {
    let mut e = Engine::load(&project(script, rooms)).unwrap_or_else(|e| panic!("{e}"));
    e.start();
    e
}

fn g(e: &Engine, n: &str) -> String {
    e.globals.get(&intern(n)).map(|v| v.to_string()).unwrap_or_else(|| "<unset>".into())
}

fn tick(e: &mut Engine, n: u32) {
    for _ in 0..n {
        e.tick();
    }
    assert!(e.errors.is_empty(), "{:?}", e.errors);
}

#[test]
fn angle_diff_sign() {
    let src = r#"
        global.a = [angle_diff(0, 90), angle_diff(90, 0), angle_diff(350, 10), angle_diff(10, 350), angle_diff(0, 180), angle_diff(0, 270)]
        global.up = angle_to(0, 0, 0, -10)
    "#;
    let e = load(src, &[("r", "width = 10\nheight = 10\n")]);
    // b − a wrapped to (−180, 180]: positive = turn counter-clockwise (towards "up" from 0)
    assert_eq!(g(&e, "a"), "[90, -90, 20, -20, 180, -90]");
    assert_eq!(g(&e, "up"), "90");
}

#[test]
fn goto_room_to_the_same_room_restarts_it_unless_persistent() {
    let src = r#"
global.creates = 0
global.room_ends = 0
object Counter {
    n = 0
    on create { global.creates += 1 }
    on step { n += 1; if n == 2 goto_room(room) }
    on room_end { global.room_ends += 1 }
}
"#;
    let room = "width = 10\nheight = 10\n[[layer]]\ntype = \"instances\"\ninstances = [ { object = \"Counter\", x = 1, y = 1 } ]\n";
    let mut e = load(src, &[("r", room)]);
    tick(&mut e, 2);
    // the room was left (room_end) and rebuilt: a fresh Counter ran create again
    assert_eq!((g(&e, "creates"), g(&e, "room_ends")), ("2".into(), "1".into()));
    let persistent = format!("persistent = true\n{room}");
    let mut e = load(src, &[("r", &persistent)]);
    tick(&mut e, 2);
    // a persistent room is saved on exit and restored on entry: same instance, no create
    assert_eq!((g(&e, "creates"), g(&e, "room_ends")), ("1".into(), "1".into()));
}

#[test]
fn draw_order_is_depth_then_creation_order() {
    let src = r#"
global.log = ""
object D {
    tag = ""
    on draw_gui { global.log += tag }
    on draw { global.dlog += tag }
}
global.dlog = ""
"#;
    let room = r#"width = 10
height = 10
[[layer]]
type = "instances"
instances = [
  { object = "D", x = 1, y = 1, vars = { tag = "a", depth = 0 } },
  { object = "D", x = 1, y = 1, vars = { tag = "b", depth = 10 } },
  { object = "D", x = 1, y = 1, vars = { tag = "c", depth = 0 } },
  { object = "D", x = 1, y = 1, vars = { tag = "d", depth = -5 } },
]
"#;
    let mut e = load(src, &[("r", room)]);
    e.render();
    // higher depth first (behind); same depth: creation order
    assert_eq!(g(&e, "log"), "bacd");
    assert_eq!(g(&e, "dlog"), "bacd", "draw_gui uses the same order as draw");
}

#[test]
fn camera_is_clamped_to_the_room_and_camera_set_stops_following() {
    let src = r#"
object Hero {
    on step {
        if global.phase == 1 { camera_set(-50, 9999) }
        if global.phase == 2 { camera_set(30, 20) }
    }
}
global.phase = 0
"#;
    // view 100×80 (display), room 300×200, hero at the far right edge
    let room = "width = 300\nheight = 200\n[camera]\nfollow = \"Hero\"\n[[layer]]\ntype = \"instances\"\ninstances = [ { object = \"Hero\", x = 299, y = 100 } ]\n";
    let mut e = load(src, &[("r", room)]);
    tick(&mut e, 1);
    assert_eq!((e.cam.x, e.cam.y), (200.0, 60.0), "following, clamped to room_width − view_w");
    e.globals.insert(intern("phase"), 1.0.into());
    tick(&mut e, 1);
    assert!(e.cam.follow.is_none(), "camera_set cancels the follow");
    assert_eq!((e.cam.x, e.cam.y), (0.0, 120.0), "camera_set positions are clamped too");
    e.globals.insert(intern("phase"), 2.0.into());
    tick(&mut e, 1);
    assert_eq!((e.cam.x, e.cam.y), (30.0, 20.0));
    // a room smaller than the view is centred (negative camera position)
    let small = "width = 60\nheight = 40\n[[layer]]\ntype = \"instances\"\ninstances = [ { object = \"Hero\", x = 1, y = 1 } ]\n";
    let mut e = load(src, &[("r", small)]);
    tick(&mut e, 1);
    assert_eq!((e.cam.x, e.cam.y), (-20.0, -20.0));
}

#[test]
fn spawn_vars_are_set_after_properties_and_before_create() {
    let src = r#"
object Bullet {
    speed_mult = 1
    on create { global.seen = speed_mult; global.dmg = damage ?? "unset" }
}
object Gun {
    on create { spawn(Bullet, 0, 0, 0, { speed_mult: 3, damage: 7 }) }
}
"#;
    let room = "width = 10\nheight = 10\n[[layer]]\ntype = \"instances\"\ninstances = [ { object = \"Gun\", x = 1, y = 1 } ]\n";
    let e = load(src, &[("r", room)]);
    assert_eq!((g(&e, "seen"), g(&e, "dmg")), ("3".into(), "7".into()));
}

#[test]
fn with_blocks_and_anonymous_functions_both_see_locals() {
    let src = r#"
object T { hp = 1 }
object Main {
    on create {
        var bonus = 5
        with (T) hp += bonus
        var add = function(v) { return v + bonus }
        global.r = add(T.hp)
    }
}
"#;
    let room = "width = 10\nheight = 10\n[[layer]]\ntype = \"instances\"\ninstances = [ { object = \"T\", x = 1, y = 1 }, { object = \"Main\", x = 1, y = 1 } ]\n";
    let e = load(src, &[("r", room)]);
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    assert_eq!(g(&e, "r"), "11");
}

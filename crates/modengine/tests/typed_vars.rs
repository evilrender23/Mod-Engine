//! Typed object variables: room values are converted and checked.

use modengine::Engine;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static N: AtomicU32 = AtomicU32::new(0);

fn project(objects: &str, room: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("modengine-typed-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("rooms")).unwrap();
    std::fs::create_dir_all(dir.join("sprites")).unwrap();
    std::fs::write(dir.join("game.toml"), "[game]\nname = \"t\"\n\n[palette]\n\"w\" = \"#ffffff\"\n").unwrap();
    std::fs::write(dir.join("sprites/dot.toml"), "pixels = ['''\nw\n''']\n").unwrap();
    std::fs::write(dir.join("objects.ms"), objects).unwrap();
    std::fs::write(dir.join("rooms/r.toml"), room).unwrap();
    dir
}

const OBJS: &str = r#"
object Base {
    walk: num(0, 10) = 2
}
object Enemy : Base {
    look: sprite = dot
    shade: color = #ffffff
    mode: list("patrol", "chase") = "patrol"
}
"#;

fn room_with(vars: &str) -> String {
    format!("width = 64\nheight = 64\n[[layer]]\ntype = \"instances\"\ninstances = [ {{ object = \"Enemy\", x = 1, y = 1, vars = {{ {vars} }} }} ]\n")
}

#[test]
fn room_values_are_converted() {
    let room = room_with(r##"walk = 7, look = "dot", shade = "#ff0000", mode = "chase""##);
    let mut e = Engine::load(&project(OBJS, &room)).unwrap_or_else(|e| panic!("{e}"));
    e.start();
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    let inst = e.insts.iter().find(|i| e.objects[i.object as usize].name == "Enemy").unwrap();
    let get = |n: &str| inst.vars.get(&modscript::intern(n)).cloned().unwrap();
    assert_eq!(get("walk").as_num(), Some(7.0));
    assert_eq!(get("shade").as_num(), Some(0xff0000 as f64));
    assert_eq!(get("look").type_name(), "sprite");
    assert_eq!(get("mode").to_string(), "chase");
}

#[test]
fn bad_room_values_are_reported() {
    let err = Engine::load(&project(OBJS, &room_with("walk = 70"))).err().expect("out of range must fail");
    assert!(err.contains("walk") && err.contains("outside"), "{err}");
    let err = Engine::load(&project(OBJS, &room_with(r#"mode = "fly""#))).err().unwrap();
    assert!(err.contains("not one of"), "{err}");
    let err = Engine::load(&project(OBJS, &room_with(r#"look = "nope""#))).err().unwrap();
    assert!(err.contains("sprite 'nope' does not exist"), "{err}");
}

#[test]
fn child_cannot_change_type() {
    let err = Engine::load(&project("object A { v: num = 1 }\nobject B : A { v: str = \"x\" }\n", "")).err().unwrap();
    assert!(err.contains("parent 'A' declares it as num"), "{err}");
    let err = Engine::load(&project("object C { x: num = 1 }", "")).err().unwrap();
    assert!(err.contains("built-in"), "{err}");
}

#[test]
fn bad_initial_value_is_a_runtime_error() {
    let room = "[[layer]]\ntype = \"instances\"\ninstances = [ { object = \"W\", x = 0, y = 0 } ]\n";
    let mut e = Engine::load(&project("object W { n: int = 1.5 }", room)).unwrap_or_else(|e| panic!("{e}"));
    e.start();
    assert!(e.errors.iter().any(|m| m.contains("W.n")), "{:?}", e.errors);
}

#[test]
fn example_free_slime_uses_its_room_values() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/platformer");
    let mut e = Engine::load(&dir).unwrap_or_else(|e| panic!("{e}"));
    e.start_room = e.rooms.iter().position(|r| r.name == "level2").unwrap();
    e.start();
    let slimes: Vec<_> = e.insts.iter().filter(|i| i.alive && e.objects[i.object as usize].name == "Slime").collect();
    let speed = |i: &modengine::instance::Instance| i.vars.get(&modscript::intern("walk_speed")).and_then(|v| v.as_num()).unwrap();
    assert!(slimes.iter().any(|i| speed(i) == 1.2), "the free slime keeps its room value");
    assert!(slimes.iter().any(|i| speed(i) == 0.5), "map slimes keep the object default");
    assert!(e.errors.is_empty(), "{:?}", e.errors);
}

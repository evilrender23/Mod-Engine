//! Terminal test hooks (docs/pruebas.md): --seed, --exec-at, --assert, --test-script,
//! --save-dir and --profile, driven headless like `modengine check` does.

use modengine::testing::{load_with_tests, EXIT_ASSERT};
use modengine::Engine;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static N: AtomicU32 = AtomicU32::new(0);

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("modengine-testflags-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const OBJS: &str = r#"
object Dice {
    rolls = []
    on create {
        randomize()
        repeat 5 push(rolls, irand(1, 6))
        global.order = [1, 2, 3, 4, 5, 6]
        shuffle(global.order)
        global.pick = choose("a", "b", "c", "d")
    }
    on step {
        push(rolls, irand(1, 100))
        if steps() == 3 save_data("slot", { n: len(rolls) })
    }
}
object Door {
    on step {
        if steps() == 5 goto_room(b)
    }
}
"#;

fn project() -> PathBuf {
    let dir = tmp("proj");
    std::fs::create_dir_all(dir.join("rooms")).unwrap();
    std::fs::write(dir.join("game.toml"), "[game]\nname = \"t\"\nstart_room = \"a\"\n").unwrap();
    std::fs::write(dir.join("objects.ms"), OBJS).unwrap();
    let room = "width = 64\nheight = 64\n[[layer]]\ntype = \"instances\"\ninstances = [ { object = \"Dice\", x = 1, y = 1 }, { object = \"Door\", x = 0, y = 0 } ]\n";
    std::fs::write(dir.join("rooms/a.toml"), room).unwrap();
    std::fs::write(dir.join("rooms/b.toml"), "width = 64\nheight = 64\n").unwrap();
    dir
}

/// What `modengine check` does: start, N steps (tick + draw), finish.
fn run(e: &mut Engine, steps: u64) -> i32 {
    e.start();
    e.test_started();
    for _ in 0..steps {
        e.test_tick();
        e.test_render();
        e.audio.clear();
    }
    e.test_finish()
}

fn state(e: &mut Engine) -> String {
    e.run_code("global.dump = str(global.order) + str(global.pick)").unwrap();
    let rolls: Vec<String> = e.insts.iter().filter(|i| e.objects[i.object as usize].name == "Dice").map(|i| i.vars.get(&modscript::intern("rolls")).map(|v| v.to_string()).unwrap_or_default()).collect();
    format!("{} {:?}", e.globals.get(&modscript::intern("dump")).map(|v| v.to_string()).unwrap_or_default(), rolls)
}

#[test]
fn seed_makes_runs_repeat_even_with_randomize() {
    let dir = project();
    let go = |seed: u64| {
        let mut e = Engine::load(&dir).unwrap();
        e.save_dir = tmp("seedsaves");
        e.set_seed(seed);
        assert_eq!(run(&mut e, 4), 0, "{:?}", e.errors);
        state(&mut e)
    };
    let (a, b, c) = (go(7), go(7), go(8));
    assert_eq!(a, b, "same seed, same game");
    assert_ne!(a, c, "another seed, another game");
}

#[test]
fn exec_at_and_asserts() {
    let dir = project();
    let mut e = Engine::load(&dir).unwrap();
    e.save_dir = tmp("asserts");
    e.add_exec_at("0:global.hp = 10").unwrap();
    e.add_exec_at("3:global.hp -= 4").unwrap();
    e.add_assert("2:global.hp == 10").unwrap();
    e.add_assert("3:global.hp == 6").unwrap();
    e.add_assert("*:global.hp > 0").unwrap();
    e.add_assert("count(Dice) == 1").unwrap();
    assert_eq!(run(&mut e, 4), 0, "{:?}", e.testing.failures);

    let mut e = Engine::load(&dir).unwrap();
    e.save_dir = tmp("asserts2");
    e.add_exec_at("0:global.hp = 1").unwrap();
    e.add_exec_at("2:global.hp = 0").unwrap();
    e.add_assert("*:global.hp > 0").unwrap();
    e.add_assert("1:global.hp == 99").unwrap();
    e.add_assert("50:true").unwrap();
    e.add_assert("nope_var + 1").unwrap();
    assert_eq!(run(&mut e, 4), EXIT_ASSERT);
    let f = e.testing.failures.join("\n");
    assert_eq!(e.testing.failures.len(), 4, "{f}");
    assert!(f.contains("en el paso 2") && f.contains("global.hp > 0"), "{f}");
    assert!(f.contains("en el paso 1") && f.contains("global.hp == 99"), "{f}");
    assert!(f.contains("no se comprobó") && f.contains("terminó en el paso 4"), "{f}");
    assert!(f.contains("al final") && f.contains("error"), "{f}");
    // bad flags are rejected before running
    assert!(e.add_exec_at("x:foo()").is_err());
    assert!(e.add_exec_at("5:if (").is_err());
    assert!(e.add_assert("3:(").is_err());
}

#[test]
fn runtime_errors_give_exit_code_1() {
    let dir = project();
    let mut e = Engine::load(&dir).unwrap();
    e.save_dir = tmp("errs");
    e.add_exec_at("2:undefined_function_xyz()").unwrap();
    assert_eq!(run(&mut e, 3), 1);
    assert_eq!(e.error_count(), 1);
}

#[test]
fn save_dir_redirects_saves() {
    let dir = project();
    let saves = tmp("saves");
    let mut e = Engine::load(&dir).unwrap();
    e.save_dir = saves.clone();
    assert_eq!(run(&mut e, 4), 0);
    assert!(saves.join("slot.json").is_file());
    assert!(!dir.join("saves").exists(), "the project's saves/ must stay untouched");
}

#[test]
fn test_scripts_spawn_in_every_room_without_duplicates() {
    let dir = project();
    let t = tmp("ts").join("probe.ms");
    std::fs::write(
        &t,
        r#"
global.probes = 0
global.rooms_seen = 0
global.watchers = 0
object Probe {
    persistent = true
    on create { global.probes += 1 }
    on room_start { global.rooms_seen += 1 }
}
object Watcher {
    on create { global.watchers += 1 }
}
object _Helper {
}
function test_rolls() { return count(Dice) }
"#,
    )
    .unwrap();
    let mut e = load_with_tests(&dir, &[t]).unwrap_or_else(|m| panic!("{m}"));
    e.save_dir = tmp("tssaves");
    e.add_assert("4:test_rolls() == 1").unwrap();
    e.add_assert("count(Probe) == 1").unwrap();
    e.add_assert("count(_Helper) == 0").unwrap();
    // a -> b at step 5 (Door), b -> a, then a restart of room a
    e.add_exec_at("8:goto_room(a)").unwrap();
    e.add_exec_at("10:goto_room(a)").unwrap();
    assert_eq!(run(&mut e, 12), 0, "{:?} {:?}", e.testing.failures, e.errors);
    let g = |n: &str| e.globals.get(&modscript::intern(n)).and_then(|v| v.as_num()).unwrap_or(-1.0);
    assert_eq!(g("probes"), 1.0, "the persistent probe is created once");
    assert_eq!(g("rooms_seen"), 4.0, "it sees every room start");
    assert_eq!(g("watchers"), 4.0, "a non-persistent test object is created again in each room");
    assert_eq!(e.rooms[e.room].name, "a");
}

#[test]
fn profile_counts_events_per_object() {
    let dir = project();
    let mut e = Engine::load(&dir).unwrap();
    e.save_dir = tmp("prof");
    e.enable_profile();
    assert_eq!(run(&mut e, 4), 0);
    let p = e.testing.profile.as_ref().unwrap();
    let dice = e.objects.iter().position(|o| o.name == "Dice").unwrap() as u32;
    let step = p.events.get(&(dice, modengine::object::Ev::Step)).unwrap();
    assert_eq!(step.calls, 4);
    assert_eq!(p.ticks, 4);
    let table = e.profile_report().unwrap();
    assert!(table.contains("Dice") && table.contains("step") && table.contains("create") && table.contains("por objeto"), "{table}");
}
